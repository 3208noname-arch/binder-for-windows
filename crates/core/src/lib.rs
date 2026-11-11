//! Windows APIにも画面にも依存しない、ファイラーの中心の処理。

mod commands;
mod entry;
mod favorites;
mod group;
mod history;
mod media;
mod path_input;
mod playback;
mod selection;
mod sort;
mod tabs;
mod tree;
mod view;
mod zoom;

pub use commands::{Bindings, Chord, Command, Gesture};
pub use entry::{Entry, SearchQuery, format_size, kind_of};
pub use favorites::{DropZone, Favorite, add_favorites, drop_zone, is_favorite, place_favorites, remove_favorite, rename_favorite};
pub use group::{Date, Group, GroupKey, Properties, date_group, name_group, size_group, value_group};
pub use history::History;
pub use media::{HOVER_DELAY_MS, HoverGate, PreviewKind, SCRUB_RESUME_MS, SCRUB_THROTTLE_MS, SPINNER_DELAY_MS, decode_text, fit_inside, fit_window, format_media_time, fraction_at, popup_size, preview_kind};
pub use path_input::parse_path_input;
pub use playback::{FALLBACK_FRAME, SPEEDS, clamp_position, next_speed, skip_seconds, speed_label, wheel_share};
pub use selection::Selection;
pub use tabs::{SavedTab, TabList, TabSession};
pub use tree::{MAX_TREE_LINES, Tree, TreeItem, TreeLine, build_tree, read_zip_items, visible_lines};
pub use view::{
    AppSettings, DEFAULT_COLUMN_WIDTH, clamp_sidebar_width, DEFAULT_ICON_SIZE, Field, FolderView, ListColumn, NewTabFolder, MAX_FIELD_WIDTH,
    MAX_DRAG_BAR_FOLDERS, MAX_FLASH_COUNT, MAX_FLASH_HOLD_MS, MAX_FLASH_MS, MAX_ICON_SIZE, MAX_SPRING_MS, MIN_FIELD_WIDTH, MIN_ICON_SIZE, MIN_SPRING_MS, ScrollbarMode, ViewMode, fit_width, rows_in_band,
    snap_flash_ms, snap_spring_ms, text_width,
};
pub use sort::{NameCmp, SortKey, SortOrder, insert_sorted, sort_entries};
pub use zoom::{ZOOM_LEVELS, clamp_offset, fit_scale, zoom_label, zoom_offset, zoom_step};
