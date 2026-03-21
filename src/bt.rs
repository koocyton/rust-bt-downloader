use anyhow::{Context, Result};
use librqbit::dht::PersistentDht;
use librqbit::dht::PersistentDhtConfig;
use librqbit::{AddTorrent, AddTorrentOptions, ManagedTorrent, Session, SessionOptions};
use librqbit::{TorrentStats, TorrentStatsState};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

use crate::engine::DownloadEvent;
use crate::task::{BtProgressExtra, TaskStatus};

type TorrentHandle = Arc<ManagedTorrent>;

/// 删除 librqbit 持久化的 DHT 路由表，下次启动会从公共 bootstrap 重新拉取。
pub fn clear_persistent_dht_cache() {
    match PersistentDht::default_persistence_filename() {
        Ok(path) => {
            if path.exists() {
                match std::fs::remove_file(&path) {
                    Ok(()) => tracing::info!("已清除 DHT 路由缓存，将重新 bootstrap: {:?}", path),
                    Err(e) => tracing::warn!("无法删除 DHT 缓存 {:?}: {}", path, e),
                }
            }
        }
        Err(e) => tracing::warn!("无法解析 DHT 缓存路径: {}", e),
    }
}

fn bt_extra_from_stats(stats: &TorrentStats) -> BtProgressExtra {
    let state_label = match stats.state {
        TorrentStatsState::Initializing => "初始化 / 解析元数据",
        TorrentStatsState::Live => "传输中",
        TorrentStatsState::Paused => "已暂停",
        TorrentStatsState::Error => "错误",
    }
    .to_string();

    let mut e = BtProgressExtra {
        uploaded: stats.uploaded_bytes,
        state_label,
        pieces_checked: stats
            .live
            .as_ref()
            .map(|l| l.snapshot.downloaded_and_checked_pieces)
            .unwrap_or(0),
        ..Default::default()
    };
    if let Some(live) = &stats.live {
        let ps = &live.snapshot.peer_stats;
        e.peer_queued = ps.queued;
        e.peer_connecting = ps.connecting;
        e.peer_live = ps.live;
        e.peer_seen = ps.seen;
        e.peer_dead = ps.dead;
        e.upload_speed_mbps = live.upload_speed.mbps;
        e.eta_label = live.time_remaining.as_ref().map(|d| format!("{}", d));
    }
    e
}

pub struct BtEngine {
    session: Arc<Session>,
    handles: Arc<Mutex<HashMap<String, TorrentHandle>>>,
    /// 结束进度监控协程（删除任务时）
    monitor_cancel: Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
}

impl BtEngine {
    pub async fn new(download_dir: std::path::PathBuf, listen_port: u16, dht_enabled: bool) -> Result<Self> {
        let dht_config = if dht_enabled {
            Some(PersistentDhtConfig::default())
        } else {
            None
        };

        let session = match Session::new_with_opts(
            download_dir.clone(),
            SessionOptions {
                disable_dht: !dht_enabled,
                disable_dht_persistence: !dht_enabled,
                dht_config,
                listen_port_range: Some(listen_port..listen_port + 100),
                ..Default::default()
            },
        )
        .await
        {
            Ok(s) => s,
            Err(e) if dht_enabled => {
                tracing::warn!("BT session with DHT failed: {}, retrying without DHT", e);
                Session::new_with_opts(
                    download_dir,
                    SessionOptions {
                        disable_dht: true,
                        disable_dht_persistence: true,
                        dht_config: None,
                        listen_port_range: Some(listen_port..listen_port + 100),
                        ..Default::default()
                    },
                )
                .await
                .context("Failed to create BT session even without DHT")?
            }
            Err(e) => return Err(e.into()),
        };

        Ok(Self {
            session,
            handles: Arc::new(Mutex::new(HashMap::new())),
            monitor_cancel: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// 全局 DHT 状态（状态栏）
    pub fn dht_status_line(&self) -> Option<String> {
        self.session.get_dht().map(|d| {
            let s = d.stats();
            format!(
                "DHT 路由 {} 节点 · 待响应 {}",
                s.routing_table_size, s.outstanding_requests
            )
        })
    }

    /// 首次添加种子并开始；若本地已有句柄则 unpause 恢复。
    pub async fn start_or_resume(
        &self,
        task_id: &str,
        url: &str,
        event_tx: mpsc::UnboundedSender<DownloadEvent>,
    ) -> Result<()> {
        {
            let handles = self.handles.lock().await;
            if let Some(h) = handles.get(task_id) {
                let h = h.clone();
                drop(handles);
                self.session.unpause(&h).await?;
                let _ = event_tx.send(DownloadEvent::StatusChanged {
                    task_id: task_id.to_string(),
                    status: TaskStatus::Downloading,
                });
                return Ok(());
            }
        }

        let add_torrent = build_add_torrent(url)?;

        let handle = self
            .session
            .add_torrent(add_torrent, Some(AddTorrentOptions::default()))
            .await
            .context("Failed to add torrent")?
            .into_handle()
            .ok_or_else(|| anyhow::anyhow!("种子重复或无法添加"))?;

        self.handles
            .lock()
            .await
            .insert(task_id.to_string(), handle.clone());

        let _ = event_tx.send(DownloadEvent::StatusChanged {
            task_id: task_id.to_string(),
            status: TaskStatus::Downloading,
        });

        let mon_cancel = tokio_util::sync::CancellationToken::new();
        self.monitor_cancel
            .lock()
            .await
            .insert(task_id.to_string(), mon_cancel.clone());

        let task_id_owned = task_id.to_string();
        let event_tx_clone = event_tx.clone();
        let handle_loop = handle.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(400));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let stats = handle_loop.stats();
                        let total = stats.total_bytes;
                        let downloaded = stats.progress_bytes;
                        let finished = stats.finished;
                        let speed = stats
                            .live
                            .as_ref()
                            .map(|l| (l.download_speed.mbps * 1024.0 * 1024.0) as u64)
                            .unwrap_or(0);
                        let peers = stats
                            .live
                            .as_ref()
                            .map(|l| l.snapshot.peer_stats.live)
                            .unwrap_or(0);

                        if total > 0 {
                            let name = handle_loop
                                .name()
                                .unwrap_or_else(|| "BT 下载".to_string());
                            let _ = event_tx_clone.send(DownloadEvent::MetadataReady {
                                task_id: task_id_owned.clone(),
                                total_size: Some(total),
                                supports_range: false,
                                filename: name,
                            });
                        }

                        let bt_extra = bt_extra_from_stats(&stats);
                        let _ = event_tx_clone.send(DownloadEvent::Progress {
                            task_id: task_id_owned.clone(),
                            downloaded,
                            speed,
                            connections: peers,
                            bt_extra: Some(bt_extra),
                        });

                        if finished {
                            let _ = event_tx_clone.send(DownloadEvent::Completed {
                                task_id: task_id_owned.clone(),
                            });
                            break;
                        }
                    }
                    _ = mon_cancel.cancelled() => break,
                }
            }
        });

        Ok(())
    }

    pub async fn pause(&self, task_id: &str) -> Result<()> {
        let h = self.handles.lock().await.get(task_id).cloned();
        if let Some(h) = h {
            self.session.pause(&h).await?;
        }
        Ok(())
    }

    pub async fn remove(&self, task_id: &str, delete_files: bool) -> Result<()> {
        if let Some(t) = self.monitor_cancel.lock().await.remove(task_id) {
            t.cancel();
        }
        let h = self.handles.lock().await.remove(task_id);
        if let Some(h) = h {
            let id = h.id();
            self.session.delete(id.into(), delete_files).await?;
        }
        Ok(())
    }
}

fn build_add_torrent(url: &str) -> Result<AddTorrent<'_>> {
    let add = if url.starts_with("magnet:") {
        AddTorrent::from_url(url)
    } else if url.ends_with(".torrent")
        && (url.starts_with("http://") || url.starts_with("https://"))
    {
        AddTorrent::from_url(url)
    } else {
        let path = Path::new(url);
        if path.exists() && path.is_file() {
            let data = std::fs::read(path)?;
            AddTorrent::from_bytes(data)
        } else {
            AddTorrent::from_url(url)
        }
    };
    Ok(add)
}

pub fn is_bt_url(url: &str) -> bool {
    if url.starts_with("magnet:") || url.contains("btih:") {
        return true;
    }
    if url.ends_with(".torrent") {
        return true;
    }
    let p = Path::new(url);
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("torrent"))
        .unwrap_or(false)
        && p.is_file()
}
