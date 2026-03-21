mod bt;
mod config;
mod engine;
mod font;
mod manager;
mod task;
mod ui;

use iced::Size;
use std::sync::{Arc, Mutex as StdMutex};
use ui::App;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    let mut app = iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size(Size::new(960.0, 640.0));

    if let Some(font_data) = font::load_chinese_font() {
        app = app.font(font_data).default_font(font::CJK_FONT);
    }

    let init_slot = Arc::new(StdMutex::new(None));
    app.run_with(|| {
        (
            App::bootstrapping(init_slot.clone()),
            App::initial_task(init_slot),
        )
    })
}
