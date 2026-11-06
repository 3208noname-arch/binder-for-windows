//! Windows APIにも画面にも依存しない、ファイラーの中心の処理。

mod entry;
mod group;
mod history;
mod sort;

pub use entry::{Entry, format_size, kind_of};
pub use group::{Date, Group, GroupKey, Properties, date_group, name_group, size_group, value_group};
pub use history::History;
pub use sort::{NameCmp, SortKey, SortOrder, insert_sorted, sort_entries};
