// release ではコンソールを開かない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod browser;
mod columns_view;
mod empty_cache;
mod favorites;
mod folder_pane;
mod icon_cache;
mod icons_view;
mod list_state;
mod list_tree;
mod list_view;
mod media_view;
mod parts;
mod path_bar;
mod quick_look;
mod scrollbar;
mod search;
mod settings_view;
mod size_cache;
mod tabs;
mod text_input;
mod theme;

use browser::Browser;
use filer_core::ViewMode;
use filer_store::Store;
use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use std::path::PathBuf;
use std::time::Instant;

/// 起動の引数で指定されたフォルダ。
fn given_path() -> Option<std::ffi::OsString> {
    std::env::args_os().skip(1).find(|arg| !arg.to_string_lossy().starts_with("--"))
}

fn start_path(given: Option<std::ffi::OsString>) -> PathBuf {
    let path: PathBuf = given.or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_else(|| "C:\\".into()).into();
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
pub fn open_store() -> Option<Store> {
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

/// panicの内容をファイルへ残す。release はコンソールが無く、ウィンドウの処理中のpanicはそのまま落ちる。
fn log_panics() {
    let Some(folder) = std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("filer")) else { return };
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let _ = std::fs::create_dir_all(&folder);
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(folder.join("panic.log")) {
            use std::io::Write;
            let _ = writeln!(file, "{info}
{backtrace}
");
        }
        default(info);
    }));
}

fn main() {
    log_panics();
    let start = Instant::now();
    let measure = std::env::args_os().any(|arg| arg == "--measure").then_some(start);
    let given = given_path();
    let restore = given.is_none();
    let path = start_path(given);
    Application::new().with_assets(parts::Assets).run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(theme::WINDOW_MIN_WIDTH), px(theme::WINDOW_MIN_HEIGHT))),
            ..Default::default()
        };
        text_input::bind_keys(cx);
        let (mode, store) = (start_mode(), open_store());
        cx.open_window(options, move |window, cx| cx.new(|cx| Browser::new(path, restore, mode, store, measure, window, cx))).unwrap();
        // 設定のウィンドウだけが残っても、本体を閉じたら終える
        cx.on_window_closed(|cx| {
            if !cx.windows().iter().any(|window| window.downcast::<Browser>().is_some()) {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
