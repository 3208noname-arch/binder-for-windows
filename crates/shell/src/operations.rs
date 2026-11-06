use std::ffi::{OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, POINT};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{
    DROPFILES, DragQueryFileW, FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR,
    FOF_NOERRORUI, FOF_RENAMEONCOLLISION, FOF_SILENT, FOFX_RECYCLEONDELETE, FileOperation, HDROP, IFileOperation,
    IShellItem, SHCreateItemFromParsingName, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

/// ファイル操作。実行はWindowsの `IFileOperation` に任せる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Copy { items: Vec<PathBuf>, destination: PathBuf },
    Move { items: Vec<PathBuf>, destination: PathBuf },
    Recycle { items: Vec<PathBuf> },
    Rename { item: PathBuf, name: String },
    NewFolder { parent: PathBuf, name: String },
}

/// 確認画面と進行表示の出し方。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// Windowsの標準の画面を出す。`hwnd` はその親のウィンドウ
    Windows { hwnd: isize },
    /// 試験用。画面を出さない
    Silent,
}

/// 操作を取り消す操作。取り消せないものは `None`。
pub fn inverse(operation: &Operation) -> Option<Operation> {
    let moved = |items: &[PathBuf], destination: &Path| -> Option<Vec<PathBuf>> {
        items.iter().map(|item| Some(destination.join(item.file_name()?))).collect()
    };
    match operation {
        Operation::Rename { item, name } => Some(Operation::Rename {
            item: item.with_file_name(name),
            name: item.file_name()?.to_string_lossy().into_owned(),
        }),
        Operation::Move { items, destination } => {
            let parent = items.first()?.parent()?.to_path_buf();
            if items.iter().any(|item| item.parent() != Some(&parent)) {
                return None;
            }
            Some(Operation::Move { items: moved(items, destination)?, destination: parent })
        }
        Operation::Copy { .. } if same_folder_copy(operation) => None,
        Operation::Copy { items, destination } => Some(Operation::Recycle { items: moved(items, destination)? }),
        Operation::Recycle { .. } | Operation::NewFolder { .. } => None,
    }
}

fn shell_item(path: &Path) -> windows::core::Result<IShellItem> {
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None) }
}

fn same_folder_copy(operation: &Operation) -> bool {
    match operation {
        Operation::Copy { items, destination } => items.iter().any(|item| item.parent() == Some(destination)),
        _ => false,
    }
}

fn queue(file_operation: &IFileOperation, operation: &Operation) -> windows::core::Result<()> {
    unsafe {
        match operation {
            Operation::Copy { items, destination } => {
                let target = shell_item(destination)?;
                for item in items {
                    file_operation.CopyItem(&shell_item(item)?, &target, None, None)?;
                }
            }
            Operation::Move { items, destination } => {
                let target = shell_item(destination)?;
                for item in items {
                    file_operation.MoveItem(&shell_item(item)?, &target, None, None)?;
                }
            }
            Operation::Recycle { items } => {
                for item in items {
                    file_operation.DeleteItem(&shell_item(item)?, None)?;
                }
            }
            Operation::Rename { item, name } => {
                file_operation.RenameItem(&shell_item(item)?, &HSTRING::from(name.as_str()), None)?;
            }
            Operation::NewFolder { parent, name } => {
                file_operation.NewItem(
                    &shell_item(parent)?,
                    FILE_ATTRIBUTE_DIRECTORY.0,
                    &HSTRING::from(name.as_str()),
                    None,
                    None,
                )?;
            }
        }
    }
    Ok(())
}

/// 操作を実行する。止めるまで戻らないので、裏のスレッドで呼ぶ。
pub fn run_operation(operation: &Operation, prompt: Prompt) -> Result<(), String> {
    run(operation, prompt).map_err(|error| error.message())
}

fn run(operation: &Operation, prompt: Prompt) -> windows::core::Result<()> {
    let mut flags = FOF_ALLOWUNDO.0 as u32 | FOFX_RECYCLEONDELETE.0;
    if prompt == Prompt::Silent {
        flags |= (FOF_SILENT.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_NOCONFIRMMKDIR.0) as u32;
    }
    if same_folder_copy(operation) {
        flags |= FOF_RENAMEONCOLLISION.0 as u32;
    }
    unsafe {
        let file_operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        file_operation.SetOperationFlags(FILEOPERATION_FLAGS(flags))?;
        if let Prompt::Windows { hwnd } = prompt {
            file_operation.SetOwnerWindow(HWND(hwnd as *mut c_void))?;
        }
        queue(&file_operation, operation)?;
        file_operation.PerformOperations()
    }
}

/// 関連付けられたアプリで開く。
pub fn open_path(path: &Path) -> Result<(), String> {
    let path = HSTRING::from(path.as_os_str());
    let result = unsafe { ShellExecuteW(None, w!("open"), &path, None, None, SW_SHOWNORMAL) };
    // 32以下は失敗の番号
    if result.0 as isize > 32 { Ok(()) } else { Err(format!("開けませんでした（{}）", result.0 as isize)) }
}

/// 新規フォルダの名前を、重ならないように決める。
pub fn unique_folder_name(parent: &Path) -> String {
    const BASE: &str = "新しいフォルダー";
    if !parent.join(BASE).exists() {
        return BASE.to_string();
    }
    (2..).map(|n| format!("{BASE} ({n})")).find(|name| !parent.join(name).exists()).expect("番号は尽きない")
}

/// 名前に使えない文字が無いか確かめる。
pub fn valid_file_name(name: &str) -> bool {
    const FORBIDDEN: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL"];
    let trimmed = name.trim();
    let stem = trimmed.split('.').next().unwrap_or("").to_uppercase();
    let numbered_device = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.len() == 4
        && stem.chars().nth(3).is_some_and(|c| c.is_ascii_digit());
    !trimmed.is_empty()
        && !trimmed.ends_with('.')
        && !name.contains(FORBIDDEN)
        && !name.chars().any(char::is_control)
        && !RESERVED.contains(&stem.as_str())
        && !numbered_device
}

fn drop_effect_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) }
}

fn global_bytes(bytes: &[u8]) -> windows::core::Result<HGLOBAL> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
        let pointer = GlobalLock(memory);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
        let _ = GlobalUnlock(memory);
        Ok(memory)
    }
}

/// `DROPFILES` の形のバイト列を作る。
fn drop_files_bytes(paths: &[PathBuf]) -> Vec<u8> {
    let header = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        pt: POINT::default(),
        fNC: false.into(),
        fWide: true.into(),
    };
    let mut bytes = unsafe {
        std::slice::from_raw_parts((&raw const header).cast::<u8>(), size_of::<DROPFILES>()).to_vec()
    };
    for path in paths {
        bytes.extend(path.as_os_str().encode_wide().chain([0]).flat_map(u16::to_le_bytes));
    }
    bytes.extend([0, 0]);
    bytes
}

/// ファイルのパスをクリップボードへ置く。`cut` は切り取り。
pub fn set_clipboard_files(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    write_clipboard(paths, cut).map_err(|error| error.message())
}

fn write_clipboard(paths: &[PathBuf], cut: bool) -> windows::core::Result<()> {
    let effect: u32 = if cut { 2 } else { 5 }; // 移動、またはコピーとリンク
    unsafe {
        OpenClipboard(None)?;
        let result = (|| {
            EmptyClipboard()?;
            let files = global_bytes(&drop_files_bytes(paths))?;
            if let Err(error) = SetClipboardData(u32::from(CF_HDROP.0), Some(HANDLE(files.0))) {
                let _ = GlobalFree(Some(files));
                return Err(error);
            }
            let effect = global_bytes(&effect.to_le_bytes())?;
            if let Err(error) = SetClipboardData(drop_effect_format(), Some(HANDLE(effect.0))) {
                let _ = GlobalFree(Some(effect));
                return Err(error);
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// クリップボードのファイルを読む。切り取りなら2番目が真。
pub fn clipboard_files() -> Option<(Vec<PathBuf>, bool)> {
    unsafe {
        IsClipboardFormatAvailable(u32::from(CF_HDROP.0)).ok()?;
        OpenClipboard(None).ok()?;
        let result = (|| {
            let files = GetClipboardData(u32::from(CF_HDROP.0)).ok()?;
            let drop = HDROP(files.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, None) as usize;
                let mut buffer = vec![0u16; length + 1];
                let copied = DragQueryFileW(drop, index, Some(&mut buffer)) as usize;
                paths.push(PathBuf::from(OsString::from_wide(&buffer[..copied])));
            }
            let cut = GetClipboardData(drop_effect_format()).ok().is_some_and(|effect| {
                let memory = HGLOBAL(effect.0);
                let pointer = GlobalLock(memory).cast::<u32>();
                let value = if pointer.is_null() { 0 } else { pointer.read_unaligned() };
                let _ = GlobalUnlock(memory);
                value & 2 != 0
            });
            Some((paths, cut))
        })();
        let _ = CloseClipboard();
        result
    }
}

