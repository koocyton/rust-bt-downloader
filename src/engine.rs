use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex};

use crate::task::{BtProgressExtra, ChunkInfo, DownloadTask, TaskStatus};

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    MetadataReady {
        task_id: String,
        total_size: Option<u64>,
        supports_range: bool,
        filename: String,
    },
    Progress {
        task_id: String,
        downloaded: u64,
        speed: u64,
        connections: usize,
        bt_extra: Option<BtProgressExtra>,
    },
    ChunkCompleted {
        task_id: String,
        chunk_index: usize,
    },
    StatusChanged {
        task_id: String,
        status: TaskStatus,
    },
    Error {
        task_id: String,
        message: String,
    },
    Completed {
        task_id: String,
    },
}

pub struct HttpDownloader {
    client: Client,
    max_connections: usize,
    chunk_size: u64,
}

impl HttpDownloader {
    pub fn new(max_connections: usize, chunk_size: u64) -> Self {
        let client = Client::builder()
            .user_agent("RustDownload/0.1 (IDM-like)")
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            max_connections,
            chunk_size,
        }
    }

    pub async fn probe_url(&self, url: &str) -> Result<(Option<u64>, bool, String)> {
        let head_result = self.client.head(url).send().await;

        let resp = match head_result {
            Ok(r) if r.status().is_success() => r,
            _ => {
                tracing::info!("HEAD request failed or rejected, falling back to GET with range header");
                let r = self
                    .client
                    .get(url)
                    .header(reqwest::header::RANGE, "bytes=0-0")
                    .send()
                    .await
                    .context("Failed to probe URL")?;
                r
            }
        };

        let status = resp.status();
        let headers = resp.headers().clone();

        let total_size = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            headers
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|v| v.parse::<u64>().ok())
        } else {
            headers
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
        };

        let supports_range = status == reqwest::StatusCode::PARTIAL_CONTENT
            || headers
                .get(reqwest::header::ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("bytes"))
                .unwrap_or(false);

        let filename = extract_filename(&headers, url);

        Ok((total_size, supports_range, filename))
    }

    pub async fn download(
        &self,
        task: &DownloadTask,
        event_tx: mpsc::UnboundedSender<DownloadEvent>,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        let (total_size, supports_range, filename) = self.probe_url(&task.url).await?;

        event_tx.send(DownloadEvent::MetadataReady {
            task_id: task.id.clone(),
            total_size,
            supports_range,
            filename: filename.clone(),
        })?;

        let save_path = task.save_path.join(&filename);

        if supports_range && total_size.is_some() {
            let total = total_size.unwrap();
            self.download_chunked(
                &task.id,
                &task.url,
                &save_path,
                total,
                &task.chunks,
                event_tx,
                cancel_token,
            )
            .await
        } else {
            self.download_single(&task.id, &task.url, &save_path, event_tx, cancel_token)
                .await
        }
    }

    async fn download_chunked(
        &self,
        task_id: &str,
        url: &str,
        save_path: &PathBuf,
        total_size: u64,
        existing_chunks: &[ChunkInfo],
        event_tx: mpsc::UnboundedSender<DownloadEvent>,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        let chunks = if existing_chunks.is_empty() {
            create_chunks(total_size, self.chunk_size)
        } else {
            existing_chunks.to_vec()
        };

        let num_connections = self.max_connections.min(chunks.len());
        let temp_dir = save_path.with_extension("rdl_parts");
        fs::create_dir_all(&temp_dir).await?;

        let downloaded = Arc::new(Mutex::new(
            chunks.iter().map(|c| c.downloaded).sum::<u64>(),
        ));
        let last_downloaded = Arc::new(Mutex::new(*downloaded.lock().await));
        let active_connections = Arc::new(Mutex::new(0usize));

        event_tx.send(DownloadEvent::StatusChanged {
            task_id: task_id.to_string(),
            status: TaskStatus::Downloading,
        })?;

        let speed_task_id = task_id.to_string();
        let speed_downloaded = downloaded.clone();
        let speed_last = last_downloaded.clone();
        let speed_tx = event_tx.clone();
        let speed_cancel = cancel_token.clone();
        let speed_connections = active_connections.clone();

        let speed_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(250));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let current = *speed_downloaded.lock().await;
                        let last = *speed_last.lock().await;
                        let delta = current.saturating_sub(last);
                        *speed_last.lock().await = current;
                        // 每 250ms 采样一次，换算为字节/秒
                        let speed = delta.saturating_mul(4);
                        let conns = *speed_connections.lock().await;
                        let _ = speed_tx.send(DownloadEvent::Progress {
                            task_id: speed_task_id.clone(),
                            downloaded: current,
                            speed,
                            connections: conns,
                            bt_extra: None,
                        });
                    }
                    _ = speed_cancel.cancelled() => break,
                }
            }
        });

        let pending_chunks: Vec<_> = chunks
            .into_iter()
            .filter(|c| !c.completed)
            .collect();

        let chunk_queue = Arc::new(Mutex::new(pending_chunks));
        let mut handles = Vec::new();

        for _ in 0..num_connections {
            let client = self.client.clone();
            let url = url.to_string();
            let task_id = task_id.to_string();
            let temp_dir = temp_dir.clone();
            let downloaded = downloaded.clone();
            let active_conns = active_connections.clone();
            let event_tx = event_tx.clone();
            let cancel = cancel_token.clone();
            let queue = chunk_queue.clone();

            let handle = tokio::spawn(async move {
                loop {
                    let chunk = {
                        let mut q = queue.lock().await;
                        q.pop()
                    };

                    let chunk = match chunk {
                        Some(c) => c,
                        None => break,
                    };

                    if cancel.is_cancelled() {
                        break;
                    }

                    {
                        let mut c = active_conns.lock().await;
                        *c += 1;
                    }

                    let result = download_chunk(
                        &client,
                        &url,
                        &chunk,
                        &temp_dir,
                        downloaded.clone(),
                        cancel.clone(),
                    )
                    .await;

                    {
                        let mut c = active_conns.lock().await;
                        *c = c.saturating_sub(1);
                    }

                    match result {
                        Ok(()) => {
                            let _ = event_tx.send(DownloadEvent::ChunkCompleted {
                                task_id: task_id.clone(),
                                chunk_index: chunk.index,
                            });
                        }
                        Err(e) => {
                            let _ = event_tx.send(DownloadEvent::Error {
                                task_id: task_id.clone(),
                                message: format!("Chunk {} failed: {}", chunk.index, e),
                            });
                        }
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            let _ = handle.await;
        }

        let was_cancelled = cancel_token.is_cancelled();
        cancel_token.cancel();
        let _ = speed_handle.await;

        if was_cancelled {
            return Ok(());
        }

        event_tx.send(DownloadEvent::StatusChanged {
            task_id: task_id.to_string(),
            status: TaskStatus::Merging,
        })?;

        merge_chunks(save_path, &temp_dir, total_size, self.chunk_size).await?;

        fs::remove_dir_all(&temp_dir).await.ok();

        event_tx.send(DownloadEvent::Completed {
            task_id: task_id.to_string(),
        })?;

        Ok(())
    }

    async fn download_single(
        &self,
        task_id: &str,
        url: &str,
        save_path: &PathBuf,
        event_tx: mpsc::UnboundedSender<DownloadEvent>,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        event_tx.send(DownloadEvent::StatusChanged {
            task_id: task_id.to_string(),
            status: TaskStatus::Downloading,
        })?;

        let resp = self.client.get(url).send().await?;
        if let Some(parent) = save_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut file = File::create(save_path).await?;
        let mut stream = resp.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_downloaded: u64 = 0;
        let mut last_time = tokio::time::Instant::now();

        while let Some(chunk_result) = stream.next().await {
            if cancel_token.is_cancelled() {
                return Ok(());
            }
            let chunk = chunk_result?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            let now = tokio::time::Instant::now();
            if now.duration_since(last_time).as_millis() >= 500 {
                let speed =
                    ((downloaded - last_downloaded) as f64 / now.duration_since(last_time).as_secs_f64()) as u64;
                last_downloaded = downloaded;
                last_time = now;
                event_tx.send(DownloadEvent::Progress {
                    task_id: task_id.to_string(),
                    downloaded,
                    speed,
                    connections: 1,
                    bt_extra: None,
                })?;
            }
        }

        file.flush().await?;

        event_tx.send(DownloadEvent::Completed {
            task_id: task_id.to_string(),
        })?;

        Ok(())
    }
}

async fn download_chunk(
    client: &Client,
    url: &str,
    chunk: &ChunkInfo,
    temp_dir: &PathBuf,
    downloaded: Arc<Mutex<u64>>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let start = chunk.start + chunk.downloaded;
    let end = chunk.end;

    if start >= end {
        return Ok(());
    }

    let range = format!("bytes={}-{}", start, end - 1);
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, range)
        .send()
        .await?;

    let chunk_path = temp_dir.join(format!("chunk_{:04}", chunk.index));
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&chunk_path)
        .await?;

    let mut stream = resp.bytes_stream();

    while let Some(data) = stream.next().await {
        if cancel.is_cancelled() {
            break;
        }
        let data = data?;
        file.write_all(&data).await?;
        let len = data.len() as u64;
        let mut d = downloaded.lock().await;
        *d += len;
    }

    file.flush().await?;
    Ok(())
}

async fn merge_chunks(
    save_path: &PathBuf,
    temp_dir: &PathBuf,
    total_size: u64,
    chunk_size: u64,
) -> Result<()> {
    if let Some(parent) = save_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut output = File::create(save_path).await?;
    let num_chunks = ((total_size + chunk_size - 1) / chunk_size) as usize;

    for i in 0..num_chunks {
        let chunk_path = temp_dir.join(format!("chunk_{:04}", i));
        if chunk_path.exists() {
            let data = fs::read(&chunk_path).await?;
            output.write_all(&data).await?;
        }
    }

    output.flush().await?;
    Ok(())
}

fn extract_filename(headers: &reqwest::header::HeaderMap, url: &str) -> String {
    if let Some(cd) = headers.get(reqwest::header::CONTENT_DISPOSITION) {
        if let Ok(cd_str) = cd.to_str() {
            if let Some(pos) = cd_str.find("filename=") {
                let name = &cd_str[pos + 9..];
                let name = name.trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
    }

    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|segments| segments.last().map(|s| s.to_string()))
        })
        .and_then(|name| {
            let decoded = urlencoding::decode(&name).unwrap_or(name.clone().into());
            let decoded = decoded.to_string();
            if decoded.is_empty() || decoded == "/" {
                None
            } else {
                Some(decoded)
            }
        })
        .unwrap_or_else(|| "download".to_string())
}

fn create_chunks(total_size: u64, chunk_size: u64) -> Vec<ChunkInfo> {
    let mut chunks = Vec::new();
    let mut start = 0u64;
    let mut index = 0;

    while start < total_size {
        let end = (start + chunk_size).min(total_size);
        chunks.push(ChunkInfo {
            index,
            start,
            end,
            downloaded: 0,
            completed: false,
        });
        start = end;
        index += 1;
    }

    chunks
}
