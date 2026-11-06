mod browser;
mod columns_view;
mod folder_pane;
mod icon_cache;
mod icons_view;
mod list_state;
mod list_tree;
mod list_view;
mod theme;

use browser::{Browser, ViewMode};
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

fn start_mode() -> ViewMode {
    let has = |flag: &str| std::env::args_os().any(|arg| arg == flag);
    if has("--columns") {
        ViewMode::Columns
    } else if has("--icons") {
        ViewMode::Icons
    } else {
        ViewMode::List
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
        let mode = start_mode();
        cx.open_window(options, move |_, cx| {
            cx.new(|cx| {
                let mut browser = Browser::new(path, measure, cx);
                browser.mode = mode;
                browser
            })
        })
        .unwrap();
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
