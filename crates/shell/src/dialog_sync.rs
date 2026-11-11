//! 標準の「開く」「保存」ダイアログを、Binderで今見ているフォルダへ移す。
//!
//! ダイアログは `explorer.exe` ではなくアプリ自身のプロセスの中に出るので、レジストリでは差し替えられない。
//! そこで、ホットキーを押された時に前面のダイアログのファイル名欄へフォルダのパスを入れ、OKを押して移動させる。
//! ホットキーは専用のスレッドで受ける。gpuiの更新処理でメッセージを回すと再入して落ちるため。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_CLICK, EnumChildWindows, GetClassNameW, GetDlgCtrlID, GetDlgItem, GetForegroundWindow, GetMessageW, GetParent, IsWindowVisible, MSG, PM_NOREMOVE, PeekMessageW,
    PostMessageW, PostThreadMessageW, SendMessageW, WM_APP, WM_GETTEXT, WM_GETTEXTLENGTH, WM_HOTKEY, WM_SETTEXT,
};
use windows::core::BOOL;

/// ホットキーの識別子。
const HOTKEY_ID: i32 = 1;
/// `Ctrl+Alt+B` の `B`。
const KEY_B: u32 = 0x42;
const MSG_ENABLE: u32 = WM_APP + 1;
const MSG_DISABLE: u32 = WM_APP + 2;
/// 標準ダイアログのクラス名。
const DIALOG_CLASS: &str = "#32770";
/// ファイル名欄（コンボボックス）のコントロールID。`cmb13`。従来の形式と、新しい「開く」の画面にある。
const FILE_NAME_ID: i32 = 0x47C;
/// 新しい「保存」の画面では、ファイル名欄の `Edit` が自分でこのIDを持つ。
const FILE_NAME_EDIT_ID: i32 = 0x3E9;
/// 「開く」「保存」ボタンのコントロールID。`IDOK`。
const OK_ID: i32 = 1;
/// OKを押してから、ファイル名欄の元の文字列を戻すまでの待ち。
const RESTORE_DELAY: Duration = Duration::from_millis(350);

/// 最後に操作したBinderの窓の、今のフォルダ。
static FOLDER: Mutex<Option<PathBuf>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(false);
static THREAD: OnceLock<u32> = OnceLock::new();

/// 標準ダイアログのクラス名か。
pub fn is_dialog_class(class: &str) -> bool {
    class == DIALOG_CLASS
}

/// ファイル名欄へ入れる文字列。長いパスの接頭辞 `\\?\` は、ダイアログが解釈しないので外す。
pub fn folder_text(folder: &Path) -> String {
    let text = folder.to_string_lossy();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

/// 移動した後に、元の文字列を戻すか。ダイアログが欄を空にした時だけ戻し、打ち直された文字列は消さない。
pub fn should_restore(before: &str, now: &str) -> bool {
    !before.is_empty() && now.is_empty()
}

/// Binderの窓が前面にある間、今のフォルダを渡す。同じ値なら何もしない。
pub fn publish_folder(folder: &Path) {
    if let Ok(mut current) = FOLDER.lock()
        && current.as_deref() != Some(folder)
    {
        *current = Some(folder.to_path_buf());
    }
}

fn current_folder() -> Option<PathBuf> {
    FOLDER.lock().ok()?.clone()
}

/// ホットキーを有効にする、または無効にする。同じ状態なら何もしない。
pub fn set_enabled(on: bool) {
    if ENABLED.swap(on, Ordering::SeqCst) == on {
        return;
    }
    let thread = if on { Some(*THREAD.get_or_init(spawn_thread)) } else { THREAD.get().copied() };
    if let Some(thread) = thread {
        let message = if on { MSG_ENABLE } else { MSG_DISABLE };
        unsafe {
            let _ = PostThreadMessageW(thread, message, WPARAM(0), LPARAM(0));
        }
    }
}

/// ホットキーを受けるスレッドを起動し、メッセージを受けられる状態になってからスレッドのIDを返す。
fn spawn_thread() -> u32 {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut message = MSG::default();
        unsafe {
            // 先にメッセージの待ち行列を作る。無いと、他のスレッドからの `PostThreadMessageW` が届かない
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            let _ = tx.send(GetCurrentThreadId());
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                match message.message {
                    MSG_ENABLE => {
                        let _ = RegisterHotKey(None, HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, KEY_B);
                    }
                    MSG_DISABLE => {
                        let _ = UnregisterHotKey(None, HOTKEY_ID);
                    }
                    WM_HOTKEY => {
                        if let Some(folder) = current_folder() {
                            send_to_foreground_dialog(&folder);
                        }
                    }
                    _ => {}
                }
            }
        }
    });
    rx.recv().unwrap_or(0)
}

fn class_name(window: HWND) -> String {
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(window, &mut buffer) };
    String::from_utf16_lossy(&buffer[..length.max(0) as usize])
}

/// 前面のウィンドウが標準ダイアログなら、フォルダへ移す。移したら `true`。
pub fn send_to_foreground_dialog(folder: &Path) -> bool {
    let dialog = unsafe { GetForegroundWindow() };
    navigate(dialog, folder)
}

/// ダイアログのファイル名欄へフォルダを入れてOKを押し、ダイアログをそのフォルダへ移す。
/// 管理者権限のアプリのダイアログなど、メッセージが届かない相手には何も起こらない。
pub fn navigate(dialog: HWND, folder: &Path) -> bool {
    if dialog.is_invalid() || !is_dialog_class(&class_name(dialog)) {
        return false;
    }
    let (Some(edit), Ok(ok)) = (find_file_name_edit(dialog), unsafe { GetDlgItem(Some(dialog), OK_ID) }) else { return false };
    let before = read_text(edit);
    write_text(edit, &folder_text(folder));
    unsafe {
        let _ = PostMessageW(Some(ok), BM_CLICK, WPARAM(0), LPARAM(0));
    }
    std::thread::sleep(RESTORE_DELAY);
    if should_restore(&before, &read_text(edit)) {
        write_text(edit, &before);
    }
    true
}

fn read_text(window: HWND) -> String {
    unsafe {
        let length = SendMessageW(window, WM_GETTEXTLENGTH, None, None).0.max(0) as usize;
        let mut buffer = vec![0u16; length + 1];
        let copied = SendMessageW(window, WM_GETTEXT, Some(WPARAM(buffer.len())), Some(LPARAM(buffer.as_mut_ptr() as isize))).0.max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied.min(length)])
    }
}

fn write_text(window: HWND, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SendMessageW(window, WM_SETTEXT, None, Some(LPARAM(wide.as_ptr() as isize)));
    }
}

/// ファイル名欄の `Edit`。欄は `ComboBoxEx32`（ID 0x47C）の中か、ID 0x3E9 の `Edit` である。
fn find_file_name_edit(dialog: HWND) -> Option<HWND> {
    struct Search {
        dialog: HWND,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(window: HWND, param: LPARAM) -> BOOL {
        let search = unsafe { &mut *(param.0 as *mut Search) };
        if class_name(window) == "Edit" && unsafe { IsWindowVisible(window) }.as_bool() && is_file_name_edit(window, search.dialog) {
            search.found = Some(window);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut search = Search { dialog, found: None };
    unsafe {
        let _ = EnumChildWindows(Some(dialog), Some(visit), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

/// 欄の `Edit` か。自分が `FILE_NAME_EDIT_ID` を持つか、親をたどった先に `FILE_NAME_ID` を持つものがある。
fn is_file_name_edit(window: HWND, dialog: HWND) -> bool {
    if unsafe { GetDlgCtrlID(window) } == FILE_NAME_EDIT_ID {
        return true;
    }
    let mut current = window;
    while current != dialog {
        if unsafe { GetDlgCtrlID(current) } == FILE_NAME_ID {
            return true;
        }
        match unsafe { GetParent(current) } {
            Ok(parent) => current = parent,
            Err(_) => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_common_dialog_class_is_a_dialog() {
        assert!(is_dialog_class("#32770"));
        assert!(!is_dialog_class("CabinetWClass"));
        assert!(!is_dialog_class("Edit"));
        assert!(!is_dialog_class(""));
    }

    #[test]
    fn the_long_path_prefix_is_removed() {
        assert_eq!(folder_text(Path::new(r"\\?\C:\Users\a b")), r"C:\Users\a b");
        assert_eq!(folder_text(Path::new(r"C:\")), r"C:\");
        assert_eq!(folder_text(Path::new(r"\\server\share\dir")), r"\\server\share\dir");
    }

    #[test]
    fn the_name_is_restored_only_into_an_empty_box() {
        assert!(should_restore("a.wav", ""));
        assert!(!should_restore("a.wav", "b.wav"));
        assert!(!should_restore("", ""));
    }

    #[test]
    fn a_missing_window_is_not_navigated() {
        assert!(!navigate(HWND::default(), Path::new(r"C:\")));
    }

    #[test]
    fn publishing_keeps_the_latest_folder() {
        publish_folder(Path::new(r"C:\one"));
        publish_folder(Path::new(r"C:\two"));
        assert_eq!(current_folder(), Some(PathBuf::from(r"C:\two")));
    }
}
