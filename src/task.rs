use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Queued,
    Downloading,
    Paused,
    /// 用户点击「停止」后，可再次开始
    Stopped,
    Completed,
    Failed,
    Merging,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskStatus::Queued => write!(f, "排队中"),
            TaskStatus::Downloading => write!(f, "下载中"),
            TaskStatus::Paused => write!(f, "已暂停"),
            TaskStatus::Stopped => write!(f, "已停止"),
            TaskStatus::Completed => write!(f, "已完成"),
            TaskStatus::Failed => write!(f, "失败"),
            TaskStatus::Merging => write!(f, "合并中"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskType {
    Http,
    BitTorrent,
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskType::Http => write!(f, "HTTP"),
            TaskType::BitTorrent => write!(f, "BT"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub completed: bool,
}

/// BT 任务运行时详情（不入库，仅内存展示）
#[derive(Debug, Clone, Default)]
pub struct BtProgressExtra {
    pub uploaded: u64,
    pub peer_queued: usize,
    pub peer_connecting: usize,
    pub peer_live: usize,
    pub peer_seen: usize,
    pub peer_dead: usize,
    pub pieces_checked: u64,
    pub state_label: String,
    pub eta_label: Option<String>,
    /// 上传估算 MiB/s（与 librqbit 内部一致）
    pub upload_speed_mbps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub save_path: PathBuf,
    pub total_size: Option<u64>,
    pub downloaded: u64,
    pub status: TaskStatus,
    pub task_type: TaskType,
    pub connections: usize,
    pub chunks: Vec<ChunkInfo>,
    pub supports_range: bool,
    pub speed: u64,
    pub created_at: DateTime<Utc>,
    pub error_message: Option<String>,
    #[serde(skip)]
    pub bt_extra: Option<BtProgressExtra>,
}

impl DownloadTask {
    pub fn new_http(url: String, filename: String, save_path: PathBuf) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            url,
            filename,
            save_path,
            total_size: None,
            downloaded: 0,
            status: TaskStatus::Queued,
            task_type: TaskType::Http,
            connections: 0,
            chunks: Vec::new(),
            supports_range: false,
            speed: 0,
            created_at: Utc::now(),
            error_message: None,
            bt_extra: None,
        }
    }

    pub fn new_bt(url: String, save_path: PathBuf) -> Self {
        let filename = "BT Download".to_string();
        Self {
            id: Uuid::new_v4().to_string(),
            url,
            filename,
            save_path,
            total_size: None,
            downloaded: 0,
            status: TaskStatus::Queued,
            task_type: TaskType::BitTorrent,
            connections: 0,
            chunks: Vec::new(),
            supports_range: false,
            speed: 0,
            created_at: Utc::now(),
            error_message: None,
            bt_extra: None,
        }
    }

    pub fn progress(&self) -> f64 {
        match self.total_size {
            Some(total) if total > 0 => (self.downloaded as f64 / total as f64) * 100.0,
            _ => 0.0,
        }
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn format_speed(bytes_per_sec: u64) -> String {
    format!("{}/s", format_size(bytes_per_sec))
}

pub fn format_eta(remaining_bytes: u64, speed: u64) -> String {
    if speed == 0 {
        return "∞".to_string();
    }
    let secs = remaining_bytes / speed;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}
