use iced::Font;
use std::borrow::Cow;
use std::path::PathBuf;

pub const CJK_FONT: Font = Font::with_name("Noto Sans CJK SC");

pub fn load_chinese_font() -> Option<Cow<'static, [u8]>> {
    let candidates = get_font_paths();

    for path in &candidates {
        if let Ok(data) = std::fs::read(path) {
            if !data.is_empty() {
                tracing::info!("Loaded Chinese font: {}", path.display());
                return Some(Cow::Owned(data));
            }
        }
    }

    tracing::warn!("No Chinese font found. Searched: {:?}", candidates);
    None
}

fn get_font_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 1. User config dir (highest priority)
    if let Some(config_dir) = dirs::config_dir() {
        paths.push(
            config_dir
                .join("rust-download")
                .join("NotoSansSC-Regular.ttf"),
        );
    }

    // 2. Next to executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("assets").join("NotoSansSC-Regular.ttf"));
            paths.push(dir.join("NotoSansSC-Regular.ttf"));
        }
    }

    // 3. Current working directory
    paths.push(PathBuf::from("assets/NotoSansSC-Regular.ttf"));

    // 4. System fonts
    #[cfg(target_os = "macos")]
    {
        paths.push("/System/Library/Fonts/Hiragino Sans GB.ttc".into());
        paths.push("/System/Library/Fonts/STHeiti Medium.ttc".into());
        paths.push("/System/Library/Fonts/STHeiti Light.ttc".into());
        paths.push("/System/Library/Fonts/PingFang.ttc".into());
        paths.push("/Library/Fonts/Arial Unicode.ttf".into());
    }

    #[cfg(target_os = "linux")]
    {
        paths.push("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc".into());
        paths.push("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc".into());
        paths.push("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc".into());
        paths.push("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc".into());
        paths.push("/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc".into());
    }

    #[cfg(target_os = "windows")]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        paths.push(format!("{}\\Fonts\\msyh.ttc", windir).into());
        paths.push(format!("{}\\Fonts\\simsun.ttc", windir).into());
        paths.push(format!("{}\\Fonts\\simhei.ttf", windir).into());
    }

    paths
}
