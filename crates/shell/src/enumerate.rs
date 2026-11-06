use filer_core::Entry;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, FILETIME};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic,
    FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
};
use windows::core::PCWSTR;

const BATCH_SIZE: usize = 2000;
const FLUSH_INTERVAL: Duration = Duration::from_millis(100);

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
    let mut last_flush = Instant::now();
    let result = loop {
        if cancelled.load(Ordering::Relaxed) {
            break Ok(());
        }
        if let Some(entry) = to_entry(&data) {
            batch.push(entry);
        }
        if should_flush(batch.len(), last_flush.elapsed()) {
            let full = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
            on_event(ListEvent::Batch(full));
            last_flush = Instant::now();
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
    let (name, raw_name) = match OsString::from_wide(&data.cFileName[..len]).into_string() {
        Ok(name) => (name, None),
        Err(raw) => (raw.to_string_lossy().into_owned(), Some(raw)),
    };
    if name == "." || name == ".." {
        return None;
    }
    let to_i64 = |t: FILETIME| (i64::from(t.dwHighDateTime) << 32) | i64::from(t.dwLowDateTime);
    Some(Entry {
        name,
        raw_name,
        is_dir: data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0,
        size: (u64::from(data.nFileSizeHigh) << 32) | u64::from(data.nFileSizeLow),
        modified: to_i64(data.ftLastWriteTime),
        created: to_i64(data.ftCreationTime),
    })
}

/// 絶対パスに `\\?\` を付ける。長いパスに加えて、末尾がドットの名前も正しく開くため。
pub fn long_path(path: &Path) -> PathBuf {
    const BACKSLASH: u16 = b'\\' as u16;
    const SLASH: u16 = b'/' as u16;
    if !path.is_absolute() {
        return path.to_path_buf();
    }
    let wide: Vec<u16> =
        path.as_os_str().encode_wide().map(|c| if c == SLASH { BACKSLASH } else { c }).collect();
    let starts = |text: &str| wide.starts_with(&text.encode_utf16().collect::<Vec<_>>());
    if starts(r"\\?\") || starts(r"\\.\") {
        return path.to_path_buf();
    }
    let (prefix, rest) = if starts(r"\\") { (r"\\?\UNC\", &wide[2..]) } else { (r"\\?\", &wide[..]) };
    let full: Vec<u16> = prefix.encode_utf16().chain(rest.iter().copied()).collect();
    OsString::from_wide(&full).into()
}

/// 溜まった項目を画面へ渡す時か。遅いNASでも、届いた分を早く見せる。
pub fn should_flush(len: usize, since_last: Duration) -> bool {
    len >= BATCH_SIZE || (len > 0 && since_last >= FLUSH_INTERVAL)
}
