//! Windows APIにも画面にも依存しない、ファイラーの中心の処理。

mod entry;
mod sort;

pub use entry::{Entry, kind_of};
pub use sort::{NameCmp, SortKey, SortOrder, insert_sorted, sort_entries};
