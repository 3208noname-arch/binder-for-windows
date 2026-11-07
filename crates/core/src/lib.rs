//! Windows APIにも画面にも依存しない、ファイラーの中心の処理。

mod entry;
mod group;
mod history;
mod selection;
mod sort;
mod view;

pub use entry::{Entry, format_size, kind_of};
pub use group::{Date, Group, GroupKey, Properties, date_group, name_group, size_group, value_group};
pub use history::History;
pub use selection::Selection;
pub use view::{
    AppSettings, DEFAULT_COLUMN_WIDTH, DEFAULT_ICON_SIZE, Field, FolderView, ListColumn, MAX_FIELD_WIDTH,
    MAX_ICON_SIZE, MAX_SPRING_MS, MIN_FIELD_WIDTH, MIN_ICON_SIZE, MIN_SPRING_MS, ScrollbarMode, ViewMode, fit_width, rows_in_band,
    snap_spring_ms, text_width,
};
pub use sort::{NameCmp, SortKey, SortOrder, insert_sorted, sort_entries};
