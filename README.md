# Rust Download Manager

**[中文](#中文说明)** · **[English](#english)**

A native GUI download manager for **HTTP/HTTPS** (multi-connection) and **BitTorrent** (magnet / `.torrent`), built with Rust, **iced**, and **librqbit**. Inspired by classic download managers (e.g. IDM-style workflow).

---

## 中文说明

### 功能概览

- **HTTP/HTTPS**：多连接分块下载、断点续传（分块临时目录）、HEAD/Range 探测、单连接回退。
- **BitTorrent**：磁力链接、HTTP(S) 种子、**本地 `.torrent` 文件**（含系统文件对话框选择）。
- **任务管理**：开始、暂停、停止、移除任务、删除任务及本地文件；任务状态持久化（`tasks.json`）。
- **界面**：深色主题、任务列表、详情面板（HTTP 显示连接与速度；BT 显示 Peers、分块、ETA、上传等）、状态栏与全局 DHT 摘要。
- **字体**：可放置 `assets/NotoSansSC-Regular.ttf` 或使用系统字体路径（见 `src/font.rs`）。

### 不支持

- **FTP**（`ftp://`）：当前仅使用 HTTP 客户端，不支持 FTP 直链；请用支持 FTP 的工具或浏览器扩展。
- **速度限制**（`speed_limit`）：配置项存在，引擎侧尚未完全接入。

### 运行

```bash
cd rust-download
cargo run --release
```

首次编译会拉取较多依赖，请耐心等待。请在**项目根目录**运行，以便加载 `assets/` 下的字体（可选）。

### 配置

配置文件位于 `~/.config/rust-download/config.json`（Windows 为 `%APPDATA%\rust-download\config.json`）。首次运行会自动创建。

| 字段 | 说明 |
|------|------|
| `download_dir` | 默认下载目录 |
| `max_connections_per_task` | 每个 HTTP 任务最大连接数 |
| `chunk_size` | HTTP 分块大小（字节） |
| `bt_listen_port` | BT 监听端口（会在范围内尝试） |
| `bt_dht_enabled` | 是否启用 DHT |
| `dht_refresh_on_startup` | 若为 `true`，启动前删除本地 DHT 路由缓存，下次从公共 bootstrap 重新填充（冷启动较慢，慎用） |

任务列表保存在同目录下的 `tasks.json`。

### 项目结构

```
src/
├── main.rs      # 入口，GUI 与初始化
├── config.rs    # 配置与路径
├── task.rs      # 任务模型与格式化
├── engine.rs    # HTTP 下载引擎
├── bt.rs        # BitTorrent（librqbit）
├── manager.rs   # 任务调度与事件
├── ui.rs        # iced 界面
└── font.rs      # 中文字体加载
```

### 开源协议

MIT

---

## English

### Overview

- **HTTP/HTTPS**: Multi-connection chunked downloads, resume via temp chunk files, HEAD/Range probing, single-stream fallback when needed.
- **BitTorrent**: Magnet links, `.torrent` over HTTP(S), **local `.torrent` files** (including native file picker via `rfd`).
- **Tasks**: Start, pause, stop, remove task, delete task + files; persistent task state (`tasks.json`).
- **UI**: Dark theme, task list, detail panel (HTTP: connections & speed; BT: peers, pieces, ETA, upload, etc.), status bar with global DHT summary.
- **Fonts**: Optional `assets/NotoSansSC-Regular.ttf` or system font paths (see `src/font.rs`).

### Not supported

- **FTP** (`ftp://`): Only HTTP is implemented for direct URLs; use an FTP-capable client.
- **Speed limit** (`speed_limit`): Field exists in config; not fully wired in the engine yet.

### Run

```bash
cd rust-download
cargo run --release
```

The first build downloads many crates. Run from the **project root** if you want bundled fonts under `assets/`.

### Configuration

`~/.config/rust-download/config.json` (on Windows: `%APPDATA%\rust-download\config.json`). Created on first launch.

| Field | Description |
|-------|-------------|
| `download_dir` | Default download directory |
| `max_connections_per_task` | Max parallel connections per HTTP task |
| `chunk_size` | HTTP chunk size in bytes |
| `bt_listen_port` | BT listen port (range is used internally) |
| `bt_dht_enabled` | Enable DHT |
| `dht_refresh_on_startup` | If `true`, delete persisted DHT routing cache before start so the next session bootstraps again (slower cold start) |

Task list: `tasks.json` in the same folder.

### Layout

```
src/
├── main.rs      # Entry, GUI bootstrap
├── config.rs    # Config & paths
├── task.rs      # Task model & formatting
├── engine.rs    # HTTP downloader
├── bt.rs        # BitTorrent (librqbit)
├── manager.rs   # Task orchestration & events
├── ui.rs        # iced UI
└── font.rs      # CJK font loading
```

### License

MIT
