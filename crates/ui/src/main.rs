mod list_state;
mod list_view;
mod theme;

use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
use list_view::ListView;
use std::path::PathBuf;
use std::time::Instant;

fn start_path() -> PathBuf {
    std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .or_else(|| std::env::var("USERPROFILE").ok())
        .unwrap_or_else(|| "C:\\".into())
        .into()
}

fn main() {
    let start = Instant::now();
    let measure = std::env::args().any(|arg| arg == "--measure");
    let path = start_path();
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        cx.open_window(options, move |_, cx| {
            cx.new(|cx| {
                let mut view = ListView::new(path, cx);
                if measure {
                    view.measure_from(start);
                }
                view
            })
        })
        .unwrap();
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
