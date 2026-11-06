use crate::long_path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
};
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::core::HSTRING;

const POLL_MS: u32 = 200;
/// 連続する変更をまとめる時間。
const SETTLE: Duration = Duration::from_millis(300);

/// フォルダの変更の監視。手放すと止まる。
pub struct WatchJob {
    cancelled: Arc<AtomicBool>,
}

impl Drop for WatchJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// フォルダの中身が変わるたびに `on_change` を呼ぶ。監視できないフォルダでは何もしない。
pub fn watch_folder(path: PathBuf, mut on_change: impl FnMut() + Send + 'static) -> WatchJob {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    std::thread::spawn(move || {
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME
            | FILE_NOTIFY_CHANGE_DIR_NAME
            | FILE_NOTIFY_CHANGE_SIZE
            | FILE_NOTIFY_CHANGE_LAST_WRITE;
        let path = HSTRING::from(long_path(&path).as_os_str());
        let Ok(handle) = (unsafe { FindFirstChangeNotificationW(&path, false, filter) }) else { return };
        while !flag.load(Ordering::Relaxed) {
            if unsafe { WaitForSingleObject(handle, POLL_MS) } != WAIT_OBJECT_0 {
                continue;
            }
            std::thread::sleep(SETTLE);
            if !flag.load(Ordering::Relaxed) {
                on_change();
            }
            if unsafe { FindNextChangeNotification(handle) }.is_err() {
                break;
            }
        }
        let _ = unsafe { FindCloseChangeNotification(handle) };
    });
    WatchJob { cancelled }
}
