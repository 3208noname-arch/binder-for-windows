//! Windowsとのやり取りをまとめる部品。

mod context_menu;
mod enumerate;
mod icons;
mod drag_wheel;
mod ime;
mod name_cmp;
mod operations;
mod properties;
mod time;
mod watch;

pub use context_menu::{MenuChoice, drag_items, preload_items_menu, show_folder_menu, show_items_menu};
pub use drag_wheel::{take_wheel, watch_wheel};
pub use ime::{control_down, cursor_position, left_button_down, place_ime_caret, remove_ime_caret};
pub use watch::{WatchJob, watch_folder};

pub use enumerate::{FolderSize, ListEvent, ListJob, folder_size, long_path, should_flush, start_list, start_search};
pub use icons::{drive_label, drives, favorites, init_thread, load_icon};
pub use name_cmp::compare_names;
pub use operations::{shortcut_folder, 
    Opened, Operation, Prompt, clipboard_files, inverse, open_target, resolve_shortcut, run_operation, run_recording_undo, set_clipboard_files, unique_folder_name, valid_file_name,
};
pub use properties::{PropertiesJob, read_properties, start_properties};
pub use time::{format_time, local_date, today};
