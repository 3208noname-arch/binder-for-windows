//! Windowsとのやり取りをまとめる部品。

mod context_menu;
mod enumerate;
mod icons;
mod ime;
mod name_cmp;
mod operations;
mod properties;
mod time;
mod watch;

pub use context_menu::{drag_items, show_folder_menu, show_items_menu};
pub use ime::{place_ime_caret, remove_ime_caret};
pub use watch::{WatchJob, watch_folder};

pub use enumerate::{FolderSize, ListEvent, ListJob, folder_size, long_path, should_flush, start_list};
pub use icons::{drives, favorites, init_thread, load_icon};
pub use name_cmp::compare_names;
pub use operations::{
    Operation, Prompt, clipboard_files, inverse, open_path, run_operation, set_clipboard_files, unique_folder_name, valid_file_name,
};
pub use properties::{PropertiesJob, read_properties, start_properties};
pub use time::{format_time, local_date, today};
