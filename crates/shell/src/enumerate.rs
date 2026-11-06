use filer_core::Entry;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic,
    FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

const BATCH_SIZE: usize = 2000;
/// これより長いパスに `\\?\` を付ける。MAX_PATHより少し手前にしている。
const LONG_PATH_LEN: usize = 240;

pub enum ListEvent {
    Batch(Vec<Entry>),
    Done,
    Failed(String),
}

/// 列挙の仕事。手放すと列挙を止める。遅いNASでも画面を待たせないよう、終了は待たない。
pub struct ListJob {
    cancelled: Arc<AtomicBool>,
}

impl Drop for ListJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub fn start_list(path: PathBuf, mut on_event: impl FnMut(ListEvent) + Send + 'static) -> ListJob {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    std::thread::spawn(move || {
        let finished = enumerate(&path, &flag, &mut on_event);
        if flag.load(Ordering::Relaxed) {
            return;
        }
        on_event(match finished {
            Ok(()) => ListEvent::Done,
            Err(e) => ListEvent::Failed(e.message()),
        });
    });
    ListJob { cancelled }
}

fn enumerate(
    path: &Path,
    cancelled: &AtomicBool,
    on_event: &mut impl FnMut(ListEvent),
) -> windows::core::Result<()> {
    let pattern: Vec<u16> = long_path(&path.join("*")).as_os_str().encode_wide().chain([0]).collect();
    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe {
        FindFirstFileExW(
            PCWSTR(pattern.as_ptr()),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    let handle = match handle {
        Ok(h) => h,
        // 何も入っていないドライブの直下はこのエラーになる
        Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Ok(()),
        Err(e) => return Err(e),
    };

    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let result = loop {
        if cancelled.load(Ordering::Relaxed) {
            break Ok(());
        }
        if let Some(entry) = to_entry(&data) {
            batch.push(entry);
        }
        if batch.len() == BATCH_SIZE {
            let full = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
            on_event(ListEvent::Batch(full));
        }
        match unsafe { FindNextFileW(handle, &mut data) } {
            Ok(()) => {}
            Err(e) if e.code() == ERROR_NO_MORE_FILES.to_hresult() => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    let _ = unsafe { FindClose(handle) };
    if !batch.is_empty() && !cancelled.load(Ordering::Relaxed) {
        on_event(ListEvent::Batch(batch));
    }
    result
}

fn to_entry(data: &WIN32_FIND_DATAW) -> Option<Entry> {
    let len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
    let name = OsString::from_wide(&data.cFileName[..len]).to_string_lossy().into_owned();
    if name == "." || name == ".." {
        return None;
    }
    let time = data.ftLastWriteTime;
    Some(Entry {
        name,
        is_dir: data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
        size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
        modified: (i64::from(time.dwHighDateTime) << 32) | i64::from(time.dwLowDateTime),
    })
}

/// MAX_PATHを超えるパスに `\\?\` を付ける。
pub fn long_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text.len() < LONG_PATH_LEN || text.starts_with(r"\\?\") {
        return path.to_path_buf();
    }
    match text.strip_prefix(r"\\") {
        Some(share) => format!(r"\\?\UNC\{share}").into(),
        None => format!(r"\\?\{text}").into(),
    }
}
