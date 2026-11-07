//! Windows APIにも画面にも依存しない、ファイラーの中心の処理。

mod commands;
mod entry;
mod group;
mod history;
mod selection;
mod sort;
mod tabs;
mod view;

pub use commands::{Bindings, Chord, Command, Gesture};
pub use entry::{Entry, SearchQuery, format_size, kind_of};
pub use group::{Date, Group, GroupKey, Properties, date_group, name_group, size_group, value_group};
pub use history::History;
pub use selection::Selection;
pub use tabs::{SavedTab, TabList, TabSession};
pub use view::{
    AppSettings, DEFAULT_COLUMN_WIDTH, DEFAULT_ICON_SIZE, Field, FolderView, ListColumn, MAX_FIELD_WIDTH,
    MAX_FLASH_MS, MAX_ICON_SIZE, MAX_SPRING_MS, MIN_FIELD_WIDTH, MIN_ICON_SIZE, MIN_SPRING_MS, ScrollbarMode, ViewMode, fit_width, rows_in_band,
    snap_flash_ms, snap_spring_ms, text_width,
};
pub use sort::{NameCmp, SortKey, SortOrder, insert_sorted, sort_entries};
