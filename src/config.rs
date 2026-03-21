use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub download_dir: PathBuf,
    pub max_concurrent_downloads: usize,
    pub max_connections_per_task: usize,
    pub chunk_size: u64,
    pub speed_limit: Option<u64>,
    pub auto_resume: bool,
    pub bt_listen_port: u16,
    pub bt_dht_enabled: bool,
    /// 为 true 时启动前删除本地 DHT 路由缓存，下次从公共 bootstrap 节点重新填充（更「新」，但冷启动略慢）
    #[serde(default)]
    pub dht_refresh_on_startup: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        let download_dir = dirs::download_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("Downloads"));

        Self {
            download_dir,
            max_concurrent_downloads: 5,
            max_connections_per_task: 8,
            chunk_size: 2 * 1024 * 1024, // 2MB
            speed_limit: None,
            auto_resume: true,
            bt_listen_port: 6881,
            bt_dht_enabled: true,
            dht_refresh_on_startup: false,
        }
    }
}

impl AppConfig {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rust-download")
            .join("config.json")
    }

    pub fn state_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rust-download")
            .join("tasks.json")
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            let config = Self::default();
            config.save()?;
            Ok(config)
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}
