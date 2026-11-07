use std::ffi::{OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, POINT};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, IPersistFile, STGM_READ};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::{
    DROPFILES, DragQueryFileW, FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR,
    FOF_NOERRORUI, FOF_RENAMEONCOLLISION, FOF_SILENT, FOF_WANTNUKEWARNING, FOFX_RECYCLEONDELETE, FileOperation, HDROP, IFileOperation,
    IShellItem, IShellLinkW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHCreateItemFromParsingName, SHELLEXECUTEINFOW, ShellExecuteExW, ShellLink,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, Interface, PCWSTR, w};

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
        Operation::Copy { items, destination } => {
            items.iter().any(|item| item.parent().is_some_and(|parent| same_path(parent, destination)))
        }
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
    run(operation, prompt).map(|_| ()).map_err(|error| error.message())
}

/// 操作を実行し、確実に元へ戻せる時だけ取り消しの操作を返す。
///
/// 名前の衝突があると、スキップや上書きの結果を確かめられない。その時に戻そうとすると
/// 別のファイルを動かすことになるので、取り消しを用意しない。
pub fn run_recording_undo(operation: &Operation, prompt: Prompt) -> Result<Option<Operation>, String> {
    let undo = if has_collision(operation) { None } else { inverse(operation) };
    let aborted = run(operation, prompt).map_err(|error| error.message())?;
    Ok(if aborted { None } else { undo })
}

/// 操作の先に、同じ名前の項目が既にあるか。
fn has_collision(operation: &Operation) -> bool {
    let exists_in = |items: &[PathBuf], destination: &Path| {
        items.iter().filter_map(|item| item.file_name()).any(|name| destination.join(name).exists())
    };
    match operation {
        Operation::Copy { items, destination } | Operation::Move { items, destination } => exists_in(items, destination),
        // 大文字と小文字だけの変更は、同じ項目なので衝突ではない
        Operation::Rename { item, name } => {
            let target = item.with_file_name(name);
            target.exists() && !same_path(&target, item)
        }
        Operation::Recycle { .. } | Operation::NewFolder { .. } => false,
    }
}

/// Windowsのパスは大文字と小文字を区別しない。
fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// 実行する。利用者が途中で止めた、またはスキップした時は真を返す。
fn run(operation: &Operation, prompt: Prompt) -> windows::core::Result<bool> {
    let mut flags = FOF_ALLOWUNDO.0 as u32 | FOFX_RECYCLEONDELETE.0;
    if prompt == Prompt::Silent {
        flags |= (FOF_SILENT.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_NOCONFIRMMKDIR.0) as u32;
    } else {
        // ごみ箱に入らない時は、完全に消す前に必ず確認を出す
        flags |= FOF_WANTNUKEWARNING.0 as u32;
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
        file_operation.PerformOperations()?;
        Ok(file_operation.GetAnyOperationsAborted()?.as_bool())
    }
}

/// 項目を開いた結果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opened {
    /// 関連付けられたアプリへ渡した
    Launched,
    /// フォルダへのショートカットだった。アプリの中で開く
    Folder(PathBuf),
}

/// ショートカットの指す先。壊れたファイルや、ファイルシステムの外を指す時は `None`。
pub fn resolve_shortcut(path: &Path) -> Option<PathBuf> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_ALL).ok()?;
        link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(path.as_os_str()), STGM_READ).ok()?;
        let mut buffer = [0u16; 32768];
        link.GetPath(&mut buffer, std::ptr::null_mut(), 0).ok()?;
        let length = buffer.iter().position(|&c| c == 0)?;
        (length > 0).then(|| PathBuf::from(OsString::from_wide(&buffer[..length])))
    }
}

/// ファイルを開く。フォルダへのショートカットはアプリの中で開けるよう、行き先を返す。
///
/// シェルは内部でウィンドウのメッセージを回すので、gpuiの更新処理の中で呼ぶと、
/// 届いた裏の処理が更新を重ねて落ちる。遅いNASでも待たされないよう、裏のスレッドで呼ぶ。
pub fn open_target(path: &Path) -> Result<Opened, String> {
    let is_link = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
    if let Some(target) = is_link.then(|| resolve_shortcut(path)).flatten().filter(|target| target.is_dir()) {
        return Ok(Opened::Folder(target));
    }
    let file = HSTRING::from(path.as_os_str());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        // スレッドがすぐ終わるので、渡し終えるまで待つ
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map(|()| Opened::Launched).map_err(|error| format!("開けませんでした（{}）", error.message()))
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
                // 1はコピー、2は移動。両方を許す値は切り取りではない
                value & 2 != 0 && value & 1 == 0
            });
            Some((paths, cut))
        })();
        let _ = CloseClipboard();
        result
    }
}

