use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::bt::{is_bt_url, BtEngine};
use crate::config::AppConfig;
use crate::engine::{DownloadEvent, HttpDownloader};
use crate::task::{DownloadTask, TaskStatus, TaskType};

pub struct DownloadManager {
    pub tasks: Arc<Mutex<Vec<DownloadTask>>>,
    config: AppConfig,
    http_downloader: Arc<HttpDownloader>,
    bt_engine: Option<Arc<BtEngine>>,
    /// 仅用于 HTTP 下载的取消；BT 由 BtEngine 管理
    cancel_tokens: Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    event_tx: mpsc::UnboundedSender<DownloadEvent>,
    pub event_rx: Option<mpsc::UnboundedReceiver<DownloadEvent>>,
}

impl DownloadManager {
    pub async fn new(config: AppConfig) -> Result<Self> {
        let http_downloader = Arc::new(HttpDownloader::new(
            config.max_connections_per_task,
            config.chunk_size,
        ));

        if config.dht_refresh_on_startup && config.bt_dht_enabled {
            crate::bt::clear_persistent_dht_cache();
        }

        let bt_engine = match BtEngine::new(
            config.download_dir.clone(),
            config.bt_listen_port,
            config.bt_dht_enabled,
        )
        .await
        {
            Ok(engine) => Some(Arc::new(engine)),
            Err(e) => {
                tracing::warn!("BT engine init failed (BT downloads disabled): {}", e);
                None
            }
        };

        let tasks = Self::load_tasks()?;
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Ok(Self {
            tasks: Arc::new(Mutex::new(tasks)),
            config,
            http_downloader,
            bt_engine,
            cancel_tokens: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            event_rx: Some(event_rx),
        })
    }

    pub async fn add_task(&self, url: String) -> Result<String> {
        let save_path = self.config.download_dir.clone();

        let task = if is_bt_url(&url) {
            DownloadTask::new_bt(url, save_path)
        } else {
            let filename = url
                .split('/')
                .last()
                .unwrap_or("download")
                .to_string();
            DownloadTask::new_http(url, filename, save_path)
        };

        let task_id = task.id.clone();

        {
            let mut tasks = self.tasks.lock().await;
            tasks.push(task);
        }

        self.save_tasks().await?;
        Ok(task_id)
    }

    pub async fn start_task(&self, task_id: &str) -> Result<()> {
        let task = {
            let tasks = self.tasks.lock().await;
            tasks.iter().find(|t| t.id == task_id).cloned()
        };

        let task = match task {
            Some(t) => t,
            None => return Err(anyhow::anyhow!("Task not found: {}", task_id)),
        };

        if task.status == TaskStatus::Downloading {
            return Ok(());
        }

        match task.task_type {
            TaskType::Http => {
                let cancel_token = tokio_util::sync::CancellationToken::new();
                {
                    let mut tokens = self.cancel_tokens.lock().await;
                    tokens.insert(task_id.to_string(), cancel_token.clone());
                }

                let downloader = self.http_downloader.clone();
                let event_tx = self.event_tx.clone();
                let cancel = cancel_token.clone();
                let task_clone = task.clone();

                tokio::spawn(async move {
                    if let Err(e) = downloader.download(&task_clone, event_tx.clone(), cancel).await
                    {
                        let _ = event_tx.send(DownloadEvent::Error {
                            task_id: task_clone.id.clone(),
                            message: e.to_string(),
                        });
                    }
                });
            }
            TaskType::BitTorrent => {
                if let Some(bt) = &self.bt_engine {
                    let bt = bt.clone();
                    let event_tx = self.event_tx.clone();
                    let url = task.url.clone();
                    let tid = task_id.to_string();
                    tokio::spawn(async move {
                        if let Err(e) = bt.start_or_resume(&tid, &url, event_tx.clone()).await {
                            let _ = event_tx.send(DownloadEvent::Error {
                                task_id: tid,
                                message: e.to_string(),
                            });
                        }
                    });
                } else {
                    return Err(anyhow::anyhow!("BT engine not available"));
                }
            }
        }

        {
            let mut tasks = self.tasks.lock().await;
            if let Some(t) = tasks.iter_mut().find(|t| t.id == task_id) {
                t.status = TaskStatus::Downloading;
            }
        }

        Ok(())
    }

    pub async fn pause_task(&self, task_id: &str) -> Result<()> {
        let task = {
            let tasks = self.tasks.lock().await;
            tasks.iter().find(|t| t.id == task_id).cloned()
        };
        let task = match task {
            Some(t) => t,
            None => return Err(anyhow::anyhow!("Task not found: {}", task_id)),
        };

        match task.task_type {
            TaskType::Http => {
                let mut tokens = self.cancel_tokens.lock().await;
                if let Some(token) = tokens.remove(task_id) {
                    token.cancel();
                }
            }
            TaskType::BitTorrent => {
                if let Some(bt) = &self.bt_engine {
                    bt.pause(task_id).await?;
                }
            }
        }

        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = TaskStatus::Paused;
            task.speed = 0;
        }

        self.save_tasks_inner(&tasks)?;
        Ok(())
    }

    /// 停止下载（任务保留，可再次开始）
    pub async fn stop_task(&self, task_id: &str) -> Result<()> {
        let task = {
            let tasks = self.tasks.lock().await;
            tasks.iter().find(|t| t.id == task_id).cloned()
        };
        let task = match task {
            Some(t) => t,
            None => return Err(anyhow::anyhow!("Task not found: {}", task_id)),
        };

        match task.task_type {
            TaskType::Http => {
                let mut tokens = self.cancel_tokens.lock().await;
                if let Some(token) = tokens.remove(task_id) {
                    token.cancel();
                }
            }
            TaskType::BitTorrent => {
                if let Some(bt) = &self.bt_engine {
                    bt.pause(task_id).await?;
                }
            }
        }

        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = TaskStatus::Stopped;
            task.speed = 0;
        }

        self.save_tasks_inner(&tasks)?;
        Ok(())
    }

    pub async fn remove_task(&self, task_id: &str, delete_file: bool) -> Result<()> {
        let task = {
            let tasks = self.tasks.lock().await;
            tasks.iter().find(|t| t.id == task_id).cloned()
        };

        let task = match task {
            Some(t) => t,
            None => return Ok(()),
        };

        match task.task_type {
            TaskType::Http => {
                let mut tokens = self.cancel_tokens.lock().await;
                if let Some(token) = tokens.remove(task_id) {
                    token.cancel();
                }
            }
            TaskType::BitTorrent => {
                if let Some(bt) = &self.bt_engine {
                    bt.remove(task_id, delete_file).await?;
                }
            }
        }

        let mut tasks = self.tasks.lock().await;
        if let Some(pos) = tasks.iter().position(|t| t.id == task_id) {
            let task = tasks.remove(pos);
            if delete_file {
                let file_path = task.save_path.join(&task.filename);
                tokio::fs::remove_file(&file_path).await.ok();
                let parts_dir = file_path.with_extension("rdl_parts");
                tokio::fs::remove_dir_all(&parts_dir).await.ok();
            }
        }

        self.save_tasks_inner(&tasks)?;
        Ok(())
    }

    /// 状态栏用：DHT 路由表规模等（无 BT 引擎时无）
    pub fn dht_status_line(&self) -> Option<String> {
        self.bt_engine.as_ref().and_then(|e| e.dht_status_line())
    }

    pub async fn handle_event(&self, event: DownloadEvent) {
        let mut tasks = self.tasks.lock().await;
        match event {
            DownloadEvent::MetadataReady {
                task_id,
                total_size,
                supports_range,
                filename,
            } => {
                if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
                    task.total_size = total_size;
                    task.supports_range = supports_range;
                    task.filename = filename;
                }
            }
            DownloadEvent::Progress {
                task_id,
                downloaded,
                speed,
                connections,
                bt_extra,
            } => {
                if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
                    if matches!(
                        task.status,
                        TaskStatus::Downloading | TaskStatus::Queued | TaskStatus::Merging
                    ) {
                        task.downloaded = downloaded;
                        task.speed = speed;
                        task.connections = connections;
                    }
                    if let Some(extra) = bt_extra {
                        task.bt_extra = Some(extra);
                    }
                }
            }
            DownloadEvent::ChunkCompleted {
                task_id,
                chunk_index,
            } => {
                if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
                    if let Some(chunk) = task.chunks.get_mut(chunk_index) {
                        chunk.completed = true;
                    }
                }
            }
            DownloadEvent::StatusChanged { task_id, status } => {
                if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
                    task.status = status;
                }
            }
            DownloadEvent::Error { task_id, message } => {
                if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
                    task.status = TaskStatus::Failed;
                    task.error_message = Some(message);
                }
            }
            DownloadEvent::Completed { task_id } => {
                if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
                    task.status = TaskStatus::Completed;
                    task.speed = 0;
                }
            }
        }
    }

    pub async fn save_tasks(&self) -> Result<()> {
        let tasks = self.tasks.lock().await;
        self.save_tasks_inner(&tasks)
    }

    fn save_tasks_inner(&self, tasks: &[DownloadTask]) -> Result<()> {
        let path = AppConfig::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(tasks)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    fn load_tasks() -> Result<Vec<DownloadTask>> {
        let path = AppConfig::state_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let mut tasks: Vec<DownloadTask> = serde_json::from_str(&content)?;
            for task in &mut tasks {
                if task.status == TaskStatus::Downloading {
                    task.status = TaskStatus::Paused;
                    task.speed = 0;
                }
            }
            Ok(tasks)
        } else {
            Ok(Vec::new())
        }
    }
}
