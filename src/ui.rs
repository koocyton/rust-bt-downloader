use iced::widget::{
    button, center, column, container, horizontal_rule, horizontal_space, progress_bar, row,
    scrollable, text, text_input, Column, Row,
};
use iced::{color, Element, Length, Subscription, Task, Theme};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex;

use crate::config::AppConfig;
use crate::engine::DownloadEvent;
use crate::manager::DownloadManager;
use crate::task::{
    format_eta, format_size, format_speed, DownloadTask, TaskStatus, TaskType,
};

// ── Dark professional palette ────────────────────────────────────
const BG_APP: iced::Color = color!(0x0D1117);
const BG_PANEL: iced::Color = color!(0x161B22);
const BG_HEADER: iced::Color = color!(0x21262D);
const BG_ROW: iced::Color = color!(0x161B22);
const BG_ROW_ALT: iced::Color = color!(0x1C2128);
const BG_HOVER: iced::Color = color!(0x30363D);
const BG_SELECTED: iced::Color = iced::Color {
    r: 0.121,
    g: 0.435,
    b: 0.984,
    a: 0.18,
};
const BG_TOOLBAR: iced::Color = color!(0x161B22);

const TEXT_PRIMARY: iced::Color = color!(0xE6EDF3);
const TEXT_SECONDARY: iced::Color = color!(0x8B949E);
const TEXT_MUTED: iced::Color = color!(0x6E7681);

const ACCENT: iced::Color = color!(0x58A6FF);
const ACCENT_HOVER: iced::Color = color!(0x79B8FF);
const ACCENT_PRESSED: iced::Color = color!(0x1F6FEB);

const GREEN: iced::Color = color!(0x3FB950);
const ORANGE: iced::Color = color!(0xD29922);
const RED: iced::Color = color!(0xF85149);
const PURPLE: iced::Color = color!(0xA371F7);

const BORDER: iced::Color = color!(0x30363D);
const PROGRESS_BG: iced::Color = color!(0x21262D);
const SPEED_COLOR: iced::Color = color!(0x56D364);

// ── Messages ─────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub enum Message {
    AppReady(Result<(), String>),
    TickTrigger,
    SyncTasks(Vec<DownloadTask>, Option<String>),
    AddPressed,
    PickTorrentFile,
    TorrentPicked(Option<PathBuf>),
    UrlInputChanged(String),
    ConfirmAdd,
    CancelAdd,
    StartTask(String),
    PauseTask(String),
    StopTask(String),
    DeleteTask(String, bool),
    SelectTask(Option<String>),
    OpenFolder,
    TaskActionDone(Result<(), String>),
}

// ── App state ────────────────────────────────────────────────────
pub struct App {
    manager: Option<Arc<DownloadManager>>,
    /// 异步初始化完成后填入，再由 `AppReady` 拆入 `manager` / `event_rx`。
    init_slot: Arc<
        StdMutex<
            Option<(
                Arc<DownloadManager>,
                tokio::sync::mpsc::UnboundedReceiver<DownloadEvent>,
            )>,
        >,
    >,
    event_rx: Option<Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<DownloadEvent>>>>,
    tasks: Vec<DownloadTask>,
    show_add_dialog: bool,
    url_input: String,
    selected_task_id: Option<String>,
    status_message: String,
    /// 全局 DHT 状态（来自 librqbit）
    dht_status: Option<String>,
}

impl App {
    pub fn bootstrapping(
        init_slot: Arc<
            StdMutex<
                Option<(
                    Arc<DownloadManager>,
                    tokio::sync::mpsc::UnboundedReceiver<DownloadEvent>,
                )>,
            >,
        >,
    ) -> Self {
        Self {
            manager: None,
            init_slot,
            event_rx: None,
            tasks: Vec::new(),
            show_add_dialog: false,
            url_input: String::new(),
            selected_task_id: None,
            status_message: "正在初始化…".to_string(),
            dht_status: None,
        }
    }

    pub fn initial_task(
        init_slot: Arc<
            StdMutex<
                Option<(
                    Arc<DownloadManager>,
                    tokio::sync::mpsc::UnboundedReceiver<DownloadEvent>,
                )>,
            >,
        >,
    ) -> Task<Message> {
        Task::perform(
            async move {
                let config = AppConfig::load().map_err(|e| e.to_string())?;
                let mut manager = DownloadManager::new(config)
                    .await
                    .map_err(|e| e.to_string())?;
                let event_rx = manager
                    .event_rx
                    .take()
                    .ok_or_else(|| "内部错误: 事件通道未就绪".to_string())?;
                *init_slot.lock().unwrap() = Some((Arc::new(manager), event_rx));
                Ok(())
            },
            Message::AppReady,
        )
    }

    pub fn title(&self) -> String {
        "Rust Download Manager".to_string()
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.manager.is_none() {
            return Subscription::none();
        }
        iced::time::every(Duration::from_millis(120)).map(|_| Message::TickTrigger)
    }

    // ── Update ───────────────────────────────────────────────────
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::AppReady(result) => match result {
                Ok(()) => {
                    let (manager, rx) = self
                        .init_slot
                        .lock()
                        .unwrap()
                        .take()
                        .expect("init bundle after successful startup");
                    self.manager = Some(manager);
                    self.event_rx = Some(Arc::new(Mutex::new(rx)));
                    self.status_message = "就绪".to_string();
                }
                Err(e) => {
                    self.status_message = format!("初始化失败: {}", e);
                }
            },
            Message::TickTrigger => {
                let Some(manager) = self.manager.clone() else {
                    return Task::none();
                };
                let Some(event_rx) = self.event_rx.clone() else {
                    return Task::none();
                };
                return Task::perform(
                    async move {
                        {
                            let mut rx = event_rx.lock().await;
                            let mut n = 0;
                            while let Ok(ev) = rx.try_recv() {
                                manager.handle_event(ev).await;
                                n += 1;
                                if n > 100 {
                                    break;
                                }
                            }
                        }
                        let tasks = manager.tasks.lock().await.clone();
                        let dht = manager.dht_status_line();
                        (tasks, dht)
                    },
                    |(tasks, dht)| Message::SyncTasks(tasks, dht),
                );
            }
            Message::SyncTasks(tasks, dht) => {
                self.tasks = tasks;
                self.dht_status = dht;
            }
            Message::AddPressed => {
                self.show_add_dialog = true;
                self.url_input.clear();
            }
            Message::PickTorrentFile => {
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(|| {
                            rfd::FileDialog::new()
                                .set_title("选择种子文件")
                                .add_filter("BitTorrent", &["torrent"])
                                .pick_file()
                        })
                        .await
                        .ok()
                        .flatten()
                    },
                    Message::TorrentPicked,
                );
            }
            Message::TorrentPicked(path) => {
                let Some(p) = path else {
                    return Task::none();
                };
                let path_str = p.display().to_string();
                let Some(mgr) = self.manager.clone() else {
                    self.status_message = "尚未就绪".to_string();
                    return Task::none();
                };
                self.status_message = format!("正在添加种子: {}", path_str);
                return Task::perform(
                    async move {
                        let id = mgr.add_task(path_str).await.map_err(|e| e.to_string())?;
                        mgr.start_task(&id).await.map_err(|e| e.to_string())?;
                        Ok(())
                    },
                    Message::TaskActionDone,
                );
            }
            Message::UrlInputChanged(v) => self.url_input = v,
            Message::ConfirmAdd => {
                let url = self.url_input.trim().to_string();
                self.show_add_dialog = false;
                self.url_input.clear();
                if !url.is_empty() {
                    let Some(mgr) = self.manager.clone() else {
                        self.status_message = "尚未就绪，请稍候".to_string();
                        return Task::none();
                    };
                    self.status_message = format!("正在添加: {}", &url);
                    return Task::perform(
                        async move {
                            let id = mgr.add_task(url).await.map_err(|e| e.to_string())?;
                            mgr.start_task(&id).await.map_err(|e| e.to_string())?;
                            Ok(())
                        },
                        Message::TaskActionDone,
                    );
                }
            }
            Message::CancelAdd => {
                self.show_add_dialog = false;
                self.url_input.clear();
            }
            Message::StartTask(id) => {
                let Some(mgr) = self.manager.clone() else {
                    return Task::none();
                };
                return Task::perform(
                    async move { mgr.start_task(&id).await.map_err(|e| e.to_string()) },
                    Message::TaskActionDone,
                );
            }
            Message::PauseTask(id) => {
                let Some(mgr) = self.manager.clone() else {
                    return Task::none();
                };
                self.status_message = "正在暂停…".to_string();
                return Task::perform(
                    async move { mgr.pause_task(&id).await.map_err(|e| e.to_string()) },
                    Message::TaskActionDone,
                );
            }
            Message::StopTask(id) => {
                let Some(mgr) = self.manager.clone() else {
                    return Task::none();
                };
                self.status_message = "正在停止…".to_string();
                return Task::perform(
                    async move { mgr.stop_task(&id).await.map_err(|e| e.to_string()) },
                    Message::TaskActionDone,
                );
            }
            Message::DeleteTask(id, delete_files) => {
                let Some(mgr) = self.manager.clone() else {
                    return Task::none();
                };
                if self.selected_task_id.as_deref() == Some(&id) {
                    self.selected_task_id = None;
                }
                self.status_message = if delete_files {
                    "正在删除任务与文件…".to_string()
                } else {
                    "正在移除任务…".to_string()
                };
                return Task::perform(
                    async move { mgr.remove_task(&id, delete_files).await.map_err(|e| e.to_string()) },
                    Message::TaskActionDone,
                );
            }
            Message::SelectTask(id) => self.selected_task_id = id,
            Message::OpenFolder => {
                let dir = self
                    .tasks
                    .first()
                    .map(|t| t.save_path.clone())
                    .unwrap_or_else(|| dirs::download_dir().unwrap_or_default());
                let _ = open::that(&dir);
            }
            Message::TaskActionDone(r) => match r {
                Ok(()) => self.status_message = "操作成功".to_string(),
                Err(e) => self.status_message = format!("错误: {}", e),
            },
        }
        Task::none()
    }

    // ── View ─────────────────────────────────────────────────────
    pub fn view(&self) -> Element<'_, Message> {
        if self.manager.is_none() {
            return container(
                center(
                    column![
                        text("正在初始化下载管理器…").size(16).color(TEXT_PRIMARY),
                        text(&self.status_message).size(13).color(TEXT_MUTED),
                    ]
                    .spacing(12)
                    .align_x(iced::Alignment::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_: &Theme| container::Style {
                background: Some(iced::Background::Color(BG_PANEL)),
                ..Default::default()
            })
            .into();
        }

        let page = column![
            self.view_toolbar(),
            horizontal_rule(1),
            self.view_task_list(),
            horizontal_rule(1),
            self.view_detail_panel(),
            horizontal_rule(1),
            self.view_status_bar(),
        ]
        .spacing(0);

        if self.show_add_dialog {
            iced::widget::stack![
                container(page)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(|_: &Theme| container::Style {
                        background: Some(iced::Background::Color(BG_APP)),
                        ..Default::default()
                    }),
                container(center(self.view_add_dialog()))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(|_: &Theme| container::Style {
                        background: Some(iced::Background::Color(iced::Color {
                            a: 0.35,
                            ..iced::Color::BLACK
                        })),
                        ..Default::default()
                    }),
            ]
            .into()
        } else {
            container(page)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Background::Color(BG_APP)),
                    ..Default::default()
                })
                .into()
        }
    }

    // ── Toolbar ──────────────────────────────────────────────────
    fn view_toolbar(&self) -> Element<'_, Message> {
        let brand = column![
            text("Rust Download").size(17).color(TEXT_PRIMARY),
            text("多连接 HTTP · BitTorrent · 磁力").size(11).color(TEXT_MUTED),
        ]
        .spacing(2);

        let add_btn = accent_button("+ 新建任务").on_press(Message::AddPressed);
        let torrent_btn = normal_button("打开种子…").on_press(Message::PickTorrentFile);
        let folder_btn = normal_button("下载目录").on_press(Message::OpenFolder);

        let active = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Downloading)
            .count();
        let speed: u64 = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Downloading)
            .map(|t| t.speed)
            .sum();

        let stats = column![
            text(format!("进行中: {}", active))
                .size(12)
                .color(TEXT_SECONDARY),
            text(format!("合计 {}", format_speed(speed)))
                .size(14)
                .color(SPEED_COLOR),
        ]
        .spacing(2)
        .align_x(iced::Alignment::End);

        container(
            row![
                brand.width(Length::Fixed(220.0)),
                horizontal_space(),
                row![add_btn, torrent_btn, folder_btn]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                horizontal_space(),
                stats,
            ]
            .align_y(iced::Alignment::Center),
        )
        .padding([14, 18])
        .width(Length::Fill)
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(BG_TOOLBAR)),
            border: iced::Border {
                color: BORDER,
                width: 0.0,
                radius: 0.into(),
            },
            ..Default::default()
        })
        .into()
    }

    // ── Task list ────────────────────────────────────────────────
    fn view_task_list(&self) -> Element<'_, Message> {
        if self.tasks.is_empty() {
            return container(
                center(
                    column![
                        text("暂无下载任务").size(18).color(TEXT_MUTED),
                        text("点击「新建下载」添加任务").size(14).color(TEXT_MUTED),
                    ]
                    .spacing(8)
                    .align_x(iced::Alignment::Center),
                )
                .width(Length::Fill)
                .height(Length::Fill),
            )
            .height(Length::FillPortion(3))
            .width(Length::Fill)
            .style(|_: &Theme| container::Style {
                background: Some(iced::Background::Color(BG_PANEL)),
                ..Default::default()
            })
            .into();
        }

        let header = container(
            row![
                text("文件名")
                    .size(12)
                    .color(TEXT_SECONDARY)
                    .width(Length::FillPortion(4)),
                text("大小")
                    .size(12)
                    .color(TEXT_SECONDARY)
                    .width(Length::FillPortion(2)),
                text("进度")
                    .size(12)
                    .color(TEXT_SECONDARY)
                    .width(Length::FillPortion(3)),
                text("速度")
                    .size(12)
                    .color(TEXT_SECONDARY)
                    .width(Length::FillPortion(2)),
                text("剩余时间")
                    .size(12)
                    .color(TEXT_SECONDARY)
                    .width(Length::FillPortion(2)),
                text("状态")
                    .size(12)
                    .color(TEXT_SECONDARY)
                    .width(Length::FillPortion(2)),
                text("操作")
                    .size(12)
                    .color(TEXT_SECONDARY)
                    .width(Length::FillPortion(3)),
            ]
            .spacing(4)
            .padding(iced::Padding::from([6, 14])),
        )
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(BG_HEADER)),
            ..Default::default()
        })
        .width(Length::Fill);

        let rows: Vec<Element<'_, Message>> =
            self.tasks.iter().map(|t| self.view_task_row(t)).collect();

        let list = scrollable(Column::with_children(rows).spacing(0)).height(Length::Fill);

        container(column![header, list].spacing(0))
            .height(Length::FillPortion(3))
            .width(Length::Fill)
            .into()
    }

    fn view_task_row<'a>(&self, task: &DownloadTask) -> Element<'a, Message> {
        let selected = self.selected_task_id.as_deref() == Some(&task.id);
        let bg = if selected { BG_SELECTED } else { BG_ROW };

        let filename = if task.filename.chars().count() > 35 {
            format!("{}…", task.filename.chars().take(34).collect::<String>())
        } else {
            task.filename.clone()
        };

        let size_str = task
            .total_size
            .map(format_size)
            .unwrap_or_else(|| "—".into());

        let progress = task.progress() as f32 / 100.0;
        let pct_str = format!("{:.1}%", task.progress());

        let active_transfer = matches!(
            task.status,
            TaskStatus::Downloading | TaskStatus::Merging
        );

        let speed_str = if active_transfer {
            format_speed(task.speed)
        } else {
            "—".into()
        };

        let speed_color = if active_transfer {
            SPEED_COLOR
        } else {
            TEXT_SECONDARY
        };

        let eta_str = if active_transfer {
            task.total_size
                .map(|t| format_eta(t.saturating_sub(task.downloaded), task.speed))
                .unwrap_or("—".into())
        } else {
            "—".into()
        };

        let (status_label, status_clr) = match task.status {
            TaskStatus::Queued => ("排队中", TEXT_MUTED),
            TaskStatus::Downloading => ("下载中", ACCENT),
            TaskStatus::Paused => ("已暂停", ORANGE),
            TaskStatus::Stopped => ("已停止", TEXT_MUTED),
            TaskStatus::Completed => ("已完成", GREEN),
            TaskStatus::Failed => ("失败", RED),
            TaskStatus::Merging => ("合并中", PURPLE),
        };

        let bar_color = match task.status {
            TaskStatus::Downloading => ACCENT,
            TaskStatus::Completed => GREEN,
            TaskStatus::Paused => ORANGE,
            TaskStatus::Stopped => TEXT_MUTED,
            TaskStatus::Merging => PURPLE,
            _ => TEXT_MUTED,
        };

        let progress_col = column![
            progress_bar(0.0..=1.0, progress)
                .height(6)
                .style(move |_: &Theme| progress_bar::Style {
                    background: iced::Background::Color(PROGRESS_BG),
                    bar: iced::Background::Color(bar_color),
                    border: iced::Border {
                        radius: 3.into(),
                        ..Default::default()
                    },
                }),
            text(pct_str).size(11).color(TEXT_SECONDARY),
        ]
        .spacing(2)
        .width(Length::FillPortion(3));

        let actions = self.build_actions(task);
        let tid = task.id.clone();

        let content = row![
            text(filename)
                .size(13)
                .color(TEXT_PRIMARY)
                .width(Length::FillPortion(4)),
            text(size_str)
                .size(13)
                .color(TEXT_SECONDARY)
                .width(Length::FillPortion(2)),
            progress_col,
            text(speed_str)
                .size(13)
                .color(speed_color)
                .width(Length::FillPortion(2)),
            text(eta_str)
                .size(13)
                .color(TEXT_SECONDARY)
                .width(Length::FillPortion(2)),
            text(status_label)
                .size(13)
                .color(status_clr)
                .width(Length::FillPortion(2)),
            actions,
        ]
        .spacing(4)
        .align_y(iced::Alignment::Center)
        .padding(iced::Padding::from([8, 14]));

        button(content)
            .on_press(Message::SelectTask(Some(tid)))
            .width(Length::Fill)
            .style(move |_: &Theme, st| {
                let c = match st {
                    button::Status::Hovered => BG_HOVER,
                    _ => bg,
                };
                button::Style {
                    background: Some(iced::Background::Color(c)),
                    text_color: TEXT_PRIMARY,
                    border: iced::Border {
                        width: 0.0,
                        radius: 0.into(),
                        color: BORDER,
                    },
                    ..Default::default()
                }
            })
            .into()
    }

    fn build_actions<'a>(&self, task: &DownloadTask) -> Element<'a, Message> {
        let id = task.id.clone();
        let mut btns: Vec<Element<'a, Message>> = Vec::new();

        match task.status {
            TaskStatus::Downloading => {
                btns.push(compact_btn("暂停", Message::PauseTask(id.clone())));
                btns.push(compact_btn("停止", Message::StopTask(id.clone())));
            }
            TaskStatus::Merging => {
                btns.push(compact_btn("停止", Message::StopTask(id.clone())));
            }
            TaskStatus::Queued | TaskStatus::Paused | TaskStatus::Stopped | TaskStatus::Failed => {
                btns.push(compact_btn("开始", Message::StartTask(id.clone())));
            }
            TaskStatus::Completed => {}
        }

        btns.push(compact_btn("移除", Message::DeleteTask(id.clone(), false)));
        btns.push(compact_btn("删文件", Message::DeleteTask(id.clone(), true)));

        Row::with_children(btns)
            .spacing(4)
            .width(Length::FillPortion(3))
            .into()
    }

    // ── Detail panel ─────────────────────────────────────────────
    fn view_detail_panel(&self) -> Element<'_, Message> {
        let sel = self
            .selected_task_id
            .as_ref()
            .and_then(|id| self.tasks.iter().find(|t| &t.id == id));

        let inner: Element<'_, Message> = if let Some(t) = sel {
            let dl = format_size(t.downloaded);
            let total = t
                .total_size
                .map(format_size)
                .unwrap_or_else(|| "未知".into());
            let url_display = if t.url.chars().count() > 70 {
                format!("{}…", t.url.chars().take(69).collect::<String>())
            } else {
                t.url.clone()
            };

            let mut rows: Vec<Element<'_, Message>> = vec![
                row![
                    label("文件:"),
                    text(&t.filename).size(12).color(TEXT_PRIMARY),
                    horizontal_space(),
                    label("类型:"),
                    text(format!("{}", t.task_type)).size(12).color(TEXT_PRIMARY),
                    horizontal_space(),
                    label("连接:"),
                    text(format!("{}", t.connections))
                        .size(12)
                        .color(TEXT_PRIMARY),
                ]
                .spacing(4)
                .into(),
                row![
                    label("路径:"),
                    text(t.save_path.display().to_string())
                        .size(12)
                        .color(TEXT_SECONDARY),
                ]
                .spacing(4)
                .into(),
                row![
                    label("进度:"),
                    text(format!("{} / {} ({:.1}%)", dl, total, t.progress()))
                        .size(12)
                        .color(TEXT_PRIMARY),
                    horizontal_space(),
                    label("速度:"),
                    text(format_speed(t.speed))
                        .size(12)
                        .color(SPEED_COLOR),
                ]
                .spacing(4)
                .into(),
                row![
                    label("地址:"),
                    text(url_display).size(12).color(TEXT_MUTED),
                ]
                .spacing(4)
                .into(),
            ];
            if t.task_type == TaskType::BitTorrent {
                if let Some(bt) = &t.bt_extra {
                    rows.push(
                        row![
                            label("BT:"),
                            text(format!(
                                "{} · 已上传 {} · 上传 {:.2} MiB/s",
                                bt.state_label,
                                format_size(bt.uploaded),
                                bt.upload_speed_mbps
                            ))
                            .size(12)
                            .color(TEXT_PRIMARY),
                        ]
                        .spacing(4)
                        .into(),
                    );
                    rows.push(
                        row![
                            label("Peers:"),
                            text(format!(
                                "排队 {} · 握手中 {} · 活跃 {} · 累计 {} · 失效 {}",
                                bt.peer_queued,
                                bt.peer_connecting,
                                bt.peer_live,
                                bt.peer_seen,
                                bt.peer_dead
                            ))
                            .size(12)
                            .color(TEXT_SECONDARY),
                        ]
                        .spacing(4)
                        .into(),
                    );
                    rows.push(
                        row![
                            label("分块:"),
                            text(format!("已校验 {} 片", bt.pieces_checked))
                                .size(12)
                                .color(TEXT_SECONDARY),
                        ]
                        .spacing(4)
                        .into(),
                    );
                    if let Some(eta) = &bt.eta_label {
                        rows.push(
                            row![
                                label("剩余:"),
                                text(eta).size(12).color(ORANGE),
                            ]
                            .spacing(4)
                            .into(),
                        );
                    }
                } else {
                    rows.push(
                        row![
                            label("BT:"),
                            text("等待统计…").size(12).color(TEXT_MUTED),
                        ]
                        .spacing(4)
                        .into(),
                    );
                }
            }
            if let Some(e) = &t.error_message {
                rows.push(
                    row![label("错误:"), text(e).size(12).color(RED)]
                        .spacing(4)
                        .into(),
                );
            }
            Column::with_children(rows).spacing(4).into()
        } else {
            column![text("选择一个任务查看详情").size(13).color(TEXT_MUTED)].into()
        };

        container(inner)
            .padding(12)
            .width(Length::Fill)
            .height(Length::Shrink)
            .style(|_: &Theme| container::Style {
                background: Some(iced::Background::Color(BG_ROW_ALT)),
                ..Default::default()
            })
            .into()
    }

    // ── Status bar ───────────────────────────────────────────────
    fn view_status_bar(&self) -> Element<'_, Message> {
        let done = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count();

        let dht_txt = if let Some(d) = &self.dht_status {
            text(d).size(11).color(TEXT_MUTED)
        } else {
            text("").size(1)
        };

        container(
            row![
                text(&self.status_message).size(12).color(TEXT_SECONDARY),
                horizontal_space(),
                dht_txt,
                horizontal_space(),
                text(format!("任务: {} / {} 已完成", done, self.tasks.len()))
                    .size(12)
                    .color(TEXT_MUTED),
            ]
            .align_y(iced::Alignment::Center),
        )
        .padding(iced::Padding::from([6, 14]))
        .width(Length::Fill)
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(BG_HEADER)),
            ..Default::default()
        })
        .into()
    }

    // ── Add-task dialog ──────────────────────────────────────────
    fn view_add_dialog(&self) -> Element<'_, Message> {
        container(
            column![
                text("新建下载任务").size(18).color(TEXT_PRIMARY),
                text("支持 HTTP/HTTPS 直链、磁力、本地 .torrent；也可使用工具栏「打开种子」")
                    .size(12)
                    .color(TEXT_MUTED),
                text_input("请输入下载链接...", &self.url_input)
                    .on_input(Message::UrlInputChanged)
                    .on_submit(Message::ConfirmAdd)
                    .padding(10)
                    .size(14),
                row![
                    horizontal_space(),
                    normal_button("取消").on_press(Message::CancelAdd),
                    accent_button("开始下载").on_press(Message::ConfirmAdd),
                ]
                .spacing(8),
            ]
            .spacing(14)
            .width(500),
        )
        .padding(24)
        .style(|_: &Theme| container::Style {
            background: Some(iced::Background::Color(BG_PANEL)),
            border: iced::Border {
                color: BORDER,
                width: 1.0,
                radius: 10.into(),
            },
            shadow: iced::Shadow {
                color: iced::Color {
                    a: 0.15,
                    ..iced::Color::BLACK
                },
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 24.0,
            },
            ..Default::default()
        })
        .into()
    }
}

// ── Reusable widgets ─────────────────────────────────────────────

fn compact_btn(label: &str, msg: Message) -> Element<'_, Message> {
    button(text(label).size(11).color(TEXT_PRIMARY))
        .padding(iced::Padding::from([4, 8]))
        .on_press(msg)
        .style(|_: &Theme, st| {
            let bg = match st {
                button::Status::Hovered => BG_HOVER,
                _ => BG_HEADER,
            };
            button::Style {
                background: Some(iced::Background::Color(bg)),
                text_color: TEXT_PRIMARY,
                border: iced::Border {
                    color: BORDER,
                    width: 1.0,
                    radius: 4.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

fn label<'a>(s: &'a str) -> iced::widget::Text<'a> {
    text(s).size(12).color(ACCENT)
}

fn accent_button(lbl: &str) -> button::Button<'_, Message> {
    button(
        text(format!("  {}  ", lbl))
            .size(13)
            .color(color!(0xFFFFFF)),
    )
    .style(|_: &Theme, st| {
        let bg = match st {
            button::Status::Hovered => ACCENT_HOVER,
            button::Status::Pressed => ACCENT_PRESSED,
            _ => ACCENT,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: color!(0xFFFFFF),
            border: iced::Border {
                radius: 6.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
}

fn normal_button(lbl: &str) -> button::Button<'_, Message> {
    button(text(format!("  {}  ", lbl)).size(13).color(TEXT_PRIMARY)).style(|_: &Theme, st| {
        let bg = match st {
            button::Status::Hovered => BG_HOVER,
            button::Status::Pressed => BG_HEADER,
            _ => BG_ROW_ALT,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: TEXT_PRIMARY,
            border: iced::Border {
                color: BORDER,
                width: 1.0,
                radius: 6.into(),
            },
            ..Default::default()
        }
    })
}

