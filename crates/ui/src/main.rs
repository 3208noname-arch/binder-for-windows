mod browser;
mod columns_view;
mod folder_pane;
mod icon_cache;
mod icons_view;
mod list_state;
mod list_tree;
mod list_view;
mod size_cache;
mod theme;

use browser::Browser;
use filer_core::ViewMode;
use filer_store::Store;
use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use std::path::PathBuf;
use std::time::Instant;

fn start_path() -> PathBuf {
    let path: PathBuf = std::env::args_os()
        .skip(1)
        .find(|arg| !arg.to_string_lossy().starts_with("--"))
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_else(|| "C:\\".into())
        .into();
    std::path::absolute(&path).unwrap_or(path)
}

/// 起動の引数で表示形式を指定した時だけ、保存済みの設定より優先する。
fn start_mode() -> Option<ViewMode> {
    let has = |flag: &str| std::env::args_os().any(|arg| arg == flag);
    if has("--columns") {
        Some(ViewMode::Columns)
    } else if has("--icons") {
        Some(ViewMode::Icons)
    } else if has("--list") {
        Some(ViewMode::List)
    } else {
        None
    }
}

/// 保存先を開く。開けない時も、保存なしで動かす。
fn open_store() -> Option<Store> {
    let file = match std::env::var_os("FILER_SETTINGS") {
        Some(file) => PathBuf::from(file),
        None => PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("filer").join("settings.sqlite"),
    };
    if let Some(folder) = file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    match Store::open(&file) {
        Ok(store) => Some(store),
        Err(error) => {
            eprintln!("設定を開けませんでした: {error}");
            None
        }
    }
}

fn main() {
    let start = Instant::now();
    let measure = std::env::args_os().any(|arg| arg == "--measure").then_some(start);
    let path = start_path();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let (mode, store) = (start_mode(), open_store());
        cx.open_window(options, move |window, cx| cx.new(|cx| Browser::new(path, mode, store, measure, window, cx))).unwrap();
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
