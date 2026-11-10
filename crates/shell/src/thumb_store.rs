//! 作ったサムネイルのディスク保存。パス、更新日時、大きさをキーにするので、ファイルが変われば別の項目になる。

use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// 保存の上限。超えたら、長く使っていないものから `TRIM_TO` まで消す。
pub const STORE_LIMIT: u64 = 512 * 1024 * 1024;
const TRIM_TO: u64 = STORE_LIMIT / 10 * 8;
/// これより長く使っていないものは、上限の内でも消す。
const MAX_AGE: Duration = Duration::from_secs(90 * 24 * 60 * 60);
/// 作れなかった印の有効期間。コーデックを後から入れた時に作り直せるよう、短くする。
const FAILED_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// 使った印の更新間隔。読むたびに書かないため。
const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

pub struct ThumbId<'a> {
    pub path: &'a Path,
    pub modified: i64,
    pub length: u64,
    pub side: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lookup {
    Picture(Vec<u8>),
    /// 以前作れなかった
    Failed,
    Missing,
}

pub struct ThumbStore {
    folder: PathBuf,
    limit: u64,
}

fn hash(path: &Path, seed: u64) -> u64 {
    path.as_os_str().encode_wide().fold(seed, |state, unit| {
        let unit = if (b'A' as u16..=b'Z' as u16).contains(&unit) { unit + 32 } else { unit };
        (state ^ unit as u64).wrapping_mul(0x0000_0100_0000_01B3)
    })
}

fn stem(id: &ThumbId) -> String {
    let first = hash(id.path, 0xCBF2_9CE4_8422_2325);
    let second = hash(id.path, 0x84222325_CBF29CE4);
    format!("{first:016x}{second:016x}-{:x}-{:x}-{}", id.modified as u64, id.length, id.side)
}

struct Stored {
    path: PathBuf,
    length: u64,
    used: SystemTime,
    failed: bool,
}

/// 消す項目を選ぶ。古いものと作れなかった印の期限切れを先に、残りは上限を超えた分だけ、使っていない順に。
fn choose_removals(mut items: Vec<Stored>, limit: u64, now: SystemTime) -> Vec<PathBuf> {
    let age = |item: &Stored| now.duration_since(item.used).unwrap_or_default();
    let target = limit / 10 * 8;
    let mut removals = Vec::new();
    items.retain(|item| {
        let expired = age(item) > if item.failed { FAILED_AGE } else { MAX_AGE };
        if expired {
            removals.push(item.path.clone());
        }
        !expired
    });
    let mut total: u64 = items.iter().map(|item| item.length).sum();
    if total > limit {
        items.sort_by_key(|item| item.used);
        for item in items {
            if total <= target {
                break;
            }
            total -= item.length;
            removals.push(item.path);
        }
    }
    removals
}

impl ThumbStore {
    pub fn new(folder: PathBuf) -> Self {
        Self::with_limit(folder, STORE_LIMIT)
    }

    pub fn with_limit(folder: PathBuf, limit: u64) -> Self {
        let _ = fs::create_dir_all(&folder);
        Self { folder, limit: limit.max(TRIM_TO.min(1)) }
    }

    fn picture_path(&self, id: &ThumbId) -> PathBuf {
        self.folder.join(format!("{}.png", stem(id)))
    }

    fn failed_path(&self, id: &ThumbId) -> PathBuf {
        self.folder.join(format!("{}.none", stem(id)))
    }

    pub fn get(&self, id: &ThumbId) -> Lookup {
        let picture = self.picture_path(id);
        if let Ok(bytes) = fs::read(&picture) {
            touch_if_old(&picture);
            return Lookup::Picture(bytes);
        }
        let marked = fs::metadata(self.failed_path(id)).and_then(|meta| meta.modified());
        match marked {
            Ok(time) if SystemTime::now().duration_since(time).unwrap_or_default() <= FAILED_AGE => Lookup::Failed,
            _ => Lookup::Missing,
        }
    }

    pub fn put(&self, id: &ThumbId, png: &[u8]) {
        self.write(&self.picture_path(id), png);
    }

    pub fn put_failed(&self, id: &ThumbId) {
        self.write(&self.failed_path(id), &[]);
    }

    /// 書きかけを読まれないよう、一時ファイルから名前を変える。
    fn write(&self, target: &Path, bytes: &[u8]) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let scratch = self.folder.join(format!("{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        if fs::write(&scratch, bytes).is_err() || fs::rename(&scratch, target).is_err() {
            let _ = fs::remove_file(&scratch);
        }
    }

    /// 上限と期限に合わせて消す。
    pub fn trim(&self) {
        let Ok(entries) = fs::read_dir(&self.folder) else { return };
        let now = SystemTime::now();
        let mut items = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(used) = meta.modified() else { continue };
            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
            if extension == "tmp" {
                // 落ちた時の書きかけ
                if now.duration_since(used).unwrap_or_default() > TOUCH_AFTER {
                    let _ = fs::remove_file(&path);
                }
                continue;
            }
            items.push(Stored { path, length: meta.len(), used, failed: extension == "none" });
        }
        for path in choose_removals(items, self.limit, now) {
            let _ = fs::remove_file(path);
        }
    }
}

fn touch_if_old(path: &Path) {
    let now = SystemTime::now();
    let old = fs::metadata(path).and_then(|meta| meta.modified()).is_ok_and(|time| now.duration_since(time).unwrap_or_default() > TOUCH_AFTER);
    if old && let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(path: &Path, modified: i64) -> ThumbId<'_> {
        ThumbId { path, modified, length: 10, side: 128 }
    }

    fn stored(name: &str, length: u64, age_hours: u64, failed: bool, now: SystemTime) -> Stored {
        Stored { path: PathBuf::from(name), length, used: now - Duration::from_secs(age_hours * 3600), failed }
    }

    #[test]
    fn a_saved_picture_comes_back_and_a_changed_file_misses() {
        let folder = tempfile::tempdir().unwrap();
        let store = ThumbStore::new(folder.path().into());
        let path = Path::new(r"Z:\a\b.mp4");
        assert_eq!(store.get(&id(path, 1)), Lookup::Missing);
        store.put(&id(path, 1), b"png");
        assert_eq!(store.get(&id(path, 1)), Lookup::Picture(b"png".to_vec()));
        assert_eq!(store.get(&id(path, 2)), Lookup::Missing);
    }

    #[test]
    fn keys_ignore_the_case_of_ascii_letters_only_where_windows_does() {
        assert_eq!(stem(&id(Path::new(r"Z:\A.MP4"), 1)), stem(&id(Path::new(r"z:\a.mp4"), 1)));
        assert_ne!(stem(&id(Path::new(r"Z:\a.mp4"), 1)), stem(&id(Path::new(r"Z:\b.mp4"), 1)));
    }

    #[test]
    fn a_failure_is_remembered() {
        let folder = tempfile::tempdir().unwrap();
        let store = ThumbStore::new(folder.path().into());
        let path = Path::new(r"Z:\broken.mp4");
        store.put_failed(&id(path, 1));
        assert_eq!(store.get(&id(path, 1)), Lookup::Failed);
    }

    #[test]
    fn over_the_limit_the_least_recently_used_go_first() {
        let now = SystemTime::now();
        let items = vec![stored("new", 50, 1, false, now), stored("old", 50, 48, false, now), stored("mid", 50, 24, false, now)];
        // 上限100の80%まで減らす
        let removed = choose_removals(items, 100, now);
        assert_eq!(removed, vec![PathBuf::from("old"), PathBuf::from("mid")]);
    }

    #[test]
    fn under_the_limit_only_expired_items_go() {
        let now = SystemTime::now();
        let hours_in = |days: u64| days * 24;
        let items = vec![
            stored("fresh", 1, 1, false, now),
            stored("stale", 1, hours_in(91), false, now),
            stored("failed-recent", 0, hours_in(6), true, now),
            stored("failed-stale", 0, hours_in(8), true, now),
        ];
        let removed = choose_removals(items, 1000, now);
        assert_eq!(removed, vec![PathBuf::from("stale"), PathBuf::from("failed-stale")]);
    }

    #[test]
    fn trim_enforces_the_limit_on_disk() {
        let folder = tempfile::tempdir().unwrap();
        let store = ThumbStore::with_limit(folder.path().into(), 100);
        for number in 0..4 {
            store.put(&id(Path::new(&format!(r"Z:\{number}.mp4")), 1), &[0u8; 50]);
        }
        store.trim();
        let left: u64 = fs::read_dir(folder.path()).unwrap().flatten().map(|e| e.metadata().unwrap().len()).sum();
        assert!(left <= 80, "left {left}");
    }
}
