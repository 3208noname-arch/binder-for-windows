use crate::init_thread;
use filer_core::Properties;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Storage::EnhancedStorage::{PKEY_Author, PKEY_Keywords, PKEY_Title};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::UI::Shell::PropertiesSystem::{
    GPS_BESTEFFORT, IPropertyStore, SHGetPropertyStoreFromParsingName,
};
use windows::core::HSTRING;

const FLUSH_INTERVAL: Duration = Duration::from_millis(150);

fn read_text(store: &IPropertyStore, key: &PROPERTYKEY) -> String {
    unsafe {
        let Ok(value) = store.GetValue(key) else { return String::new() };
        let Ok(text) = PropVariantToStringAlloc(&value) else { return String::new() };
        let result = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const _));
        result
    }
}

/// 作成者、タグ、タイトルを読む。読めない項目は空のまま返す。
pub fn read_properties(path: &Path) -> Properties {
    let store: Result<IPropertyStore, _> = unsafe {
        SHGetPropertyStoreFromParsingName(&HSTRING::from(path.as_os_str()), None, GPS_BESTEFFORT)
    };
    let Ok(store) = store else { return Properties::default() };
    Properties {
        author: read_text(&store, &PKEY_Author),
        tags: read_text(&store, &PKEY_Keywords),
        title: read_text(&store, &PKEY_Title),
    }
}

/// 裏のスレッドでプロパティを読む仕事。手放すと止まる。
pub struct PropertiesJob {
    cancelled: Arc<AtomicBool>,
}

impl Drop for PropertiesJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub fn start_properties(
    folder: PathBuf,
    names: Vec<OsString>,
    mut on_batch: impl FnMut(Vec<(OsString, Properties)>) + Send + 'static,
) -> PropertiesJob {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    std::thread::spawn(move || {
        init_thread();
        let mut batch = Vec::new();
        let mut last_flush = Instant::now();
        for name in names {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            let properties = read_properties(&folder.join(&name));
            batch.push((name, properties));
            if last_flush.elapsed() >= FLUSH_INTERVAL {
                on_batch(std::mem::take(&mut batch));
                last_flush = Instant::now();
            }
        }
        if !flag.load(Ordering::Relaxed) {
            on_batch(batch);
        }
    });
    PropertiesJob { cancelled }
}
