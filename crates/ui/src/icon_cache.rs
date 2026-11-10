use filer_core::Entry;
use filer_shell::{Thumb, ThumbJob, ThumbKind, ThumbPool, ThumbStore, init_thread, load_icon};
use futures::StreamExt;
use gpui::{Context, Image, ImageFormat, Task};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

/// アイコンの大きさ（ピクセル）。
pub const SMALL: u32 = 32;
pub const LARGE: u32 = 128;
pub const HUGE: u32 = 256;

/// 1件ごとに違う絵を持つ種類。これ以外は拡張子で共有する。
const PER_FILE: &[&str] = &["exe", "lnk", "ico", "url"];
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "heic"];
const VIDEOS: &[&str] = &["mp4", "mov", "mkv", "avi", "webm"];

/// 順番待ちの上限。超えたら古い要求（もう見えていない行）から捨てる。
const MAX_PENDING: usize = 512;
const MAX_RETRIES: u8 = 3;
/// 届かなかったファイルのサムネイルを取り直すまでの間隔。切れたNASを叩き続けないため。
const RETRY_AFTER: Duration = Duration::from_secs(30);

struct Request {
    key: String,
    path: PathBuf,
    size: u32,
}

enum Loaded {
    Done(Option<Vec<u8>>),
    /// 順番待ちが溜まって捨てた要求。見えていれば次の描画で取り直す
    Dropped,
    /// ファイルに届かなかった。時間を置いて取り直す
    Later,
}

/// 取得中と失敗は `None` で覚える。1件の失敗で同じ種類の全部が仮の図形にならないよう、
/// 種類で共有する絵だけは別のファイルで数回取り直す。
pub struct IconCache {
    icons: HashMap<String, Option<Arc<Image>>>,
    failures: HashMap<String, u8>,
    /// 同じ絵のフォルダが多いので、絵の中身で1つにまとめる
    images: HashMap<u64, Arc<Image>>,
    /// 普通のフォルダの絵を取る先
    plain_folder: PathBuf,
    requests: Sender<Request>,
    thumbnails: ThumbPool,
    /// 描画の途中で見えたサムネイル。描画が終わったら、これ以外の要求を取り消す
    visible: HashSet<String>,
    drawing: bool,
    retry_at: HashMap<String, Instant>,
    _receiver: Task<()>,
}

fn thumb_kind(extension: &str) -> Option<ThumbKind> {
    if IMAGES.contains(&extension) {
        Some(ThumbKind::Image)
    } else if VIDEOS.contains(&extension) {
        Some(ThumbKind::Video)
    } else {
        None
    }
}

/// 保存先。`FILER_THUMBNAILS` で試験用に分けられる。
fn thumbnail_store() -> Option<ThumbStore> {
    let folder = match std::env::var_os("FILER_THUMBNAILS") {
        Some(folder) => PathBuf::from(folder),
        None => crate::data_folder()?.join("thumbnails").join("v1"),
    };
    Some(ThumbStore::new(folder))
}

fn may_retry(key: &str, failures: u8) -> bool {
    let shared = key.starts_with('.') || key.starts_with("<folder>");
    shared && failures < MAX_RETRIES
}

impl IconCache {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (requests, queue) = channel::<Request>();
        let (done, mut results) = futures::channel::mpsc::unbounded();
        let thumb_done = done.clone();
        let thumbnails = ThumbPool::start(thumbnail_store(), move |key, thumb| {
            let loaded = match thumb {
                Thumb::Picture(bytes) => Loaded::Done(Some(bytes)),
                Thumb::Unavailable => Loaded::Done(None),
                Thumb::Later => Loaded::Later,
                Thumb::Cancelled => Loaded::Dropped,
            };
            let _ = thumb_done.unbounded_send((key, loaded));
        });
        std::thread::spawn(move || {
            init_thread();
            let mut pending = Vec::new();
            while let Ok(first) = queue.recv() {
                pending.push(first);
                // 新しい要求ほど今見えている行なので、後ろから取る
                while let Some(request) = pending.pop() {
                    let png = load_icon(&request.path, request.size);
                    if done.unbounded_send((request.key, Loaded::Done(png))).is_err() {
                        return;
                    }
                    pending.extend(queue.try_iter());
                    let excess = pending.len().saturating_sub(MAX_PENDING);
                    for old in pending.drain(..excess) {
                        let _ = done.unbounded_send((old.key, Loaded::Dropped));
                    }
                }
            }
        });
        let receiver = cx.spawn(async move |this, cx| {
            while let Some((key, loaded)) = results.next().await {
                let stored = this.update(cx, |cache, cx| {
                    cache.receive(key, loaded);
                    cx.notify();
                });
                if stored.is_err() {
                    break;
                }
            }
        });
        Self { icons: HashMap::new(), failures: HashMap::new(), images: HashMap::new(), plain_folder: std::env::temp_dir(), requests, thumbnails, visible: HashSet::new(), drawing: false, retry_at: HashMap::new(), _receiver: receiver }
    }

    fn receive(&mut self, key: String, loaded: Loaded) {
        match loaded {
            Loaded::Done(Some(bytes)) => {
                let image = Image::from_bytes(ImageFormat::Png, bytes);
                let image = self.images.entry(image.id()).or_insert_with(|| Arc::new(image)).clone();
                self.icons.insert(key, Some(image));
            }
            Loaded::Done(None) => {
                let failures = self.failures.entry(key.clone()).or_default();
                *failures += 1;
                if may_retry(&key, *failures) {
                    self.icons.remove(&key);
                }
            }
            Loaded::Dropped => {
                self.icons.remove(&key);
            }
            Loaded::Later => {
                self.retry_at.insert(key, Instant::now() + RETRY_AFTER);
            }
        }
    }

    /// 持っていれば返す。無ければ裏で取りに行き、届いたら画面を更新する。
    /// 同じ描画で呼ばれなかったサムネイルの要求は、描画の後で取り消す。
    pub fn get(&mut self, folder: &Path, entry: &Entry, size: u32, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        if !self.drawing {
            self.drawing = true;
            let this = cx.entity();
            cx.defer(move |cx| this.update(cx, |cache, _| cache.finish_drawing()));
        }
        self.lookup(folder, entry, size, false)
    }

    /// 一覧と別の窓から使う。一覧の描画で取り消されない。
    pub fn get_kept(&mut self, folder: &Path, entry: &Entry, size: u32) -> Option<Arc<Image>> {
        self.lookup(folder, entry, size, true)
    }

    fn finish_drawing(&mut self) {
        self.drawing = false;
        let visible = std::mem::take(&mut self.visible);
        for key in self.thumbnails.keep_only(&visible) {
            self.icons.remove(&key);
        }
    }

    fn lookup(&mut self, folder: &Path, entry: &Entry, size: u32, keep: bool) -> Option<Arc<Image>> {
        if entry.is_dir {
            return self.folder(&folder.join(entry.os_name()), size);
        }
        let extension = entry.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        let e = extension.as_str();
        let path = folder.join(entry.os_name());
        let kind = thumb_kind(e).filter(|_| size >= LARGE && entry.size > 0);
        let shared_key = format!(".{e}:{size}");
        if let Some(kind) = kind {
            let own = self.thumbnail(&path, entry, size, kind, keep);
            return own.or_else(|| self.fetch(shared_key, path, size));
        }
        let key = if PER_FILE.contains(&e) { format!("{}:{size}", path.display()) } else { shared_key };
        self.fetch(key, path, size)
    }

    /// 自分の絵が無い間と作れない時は `None` を返し、呼び出し側が種類のアイコンを出す。
    fn thumbnail(&mut self, path: &Path, entry: &Entry, side: u32, kind: ThumbKind, keep: bool) -> Option<Arc<Image>> {
        // 更新日時と大きさを含めて、ファイルが変わったら別の絵として取り直す
        let key = format!("{}|{:x}|{:x}:{side}", path.display(), entry.modified, entry.size);
        if !keep {
            self.visible.insert(key.clone());
        }
        if self.retry_at.get(&key).is_some_and(|at| *at <= Instant::now()) {
            self.retry_at.remove(&key);
            self.icons.remove(&key);
        }
        if let Some(icon) = self.icons.get(&key) {
            return icon.clone();
        }
        self.icons.insert(key.clone(), None);
        self.thumbnails.request(ThumbJob { key, path: path.to_path_buf(), side, kind, modified: entry.modified, length: entry.size, keep });
        None
    }

    /// フォルダの絵。フォルダごとに変えられるので1件ずつ取り、届くまでは普通のフォルダの絵を出す。
    pub fn folder(&mut self, path: &Path, size: u32) -> Option<Arc<Image>> {
        let own = self.fetch(format!("{}:{size}", path.display()), path.to_path_buf(), size);
        own.or_else(|| self.fetch(format!("<folder>:{size}"), self.plain_folder.clone(), size))
    }

    fn fetch(&mut self, key: String, path: PathBuf, size: u32) -> Option<Arc<Image>> {
        if let Some(icon) = self.icons.get(&key) {
            return icon.clone();
        }
        self.icons.insert(key.clone(), None);
        let _ = self.requests.send(Request { key, path, size });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{ThumbKind, may_retry, thumb_kind};

    #[test]
    fn pictures_and_videos_are_told_apart() {
        assert_eq!(thumb_kind("heic"), Some(ThumbKind::Image));
        assert_eq!(thumb_kind("mkv"), Some(ThumbKind::Video));
        assert_eq!(thumb_kind("pdf"), None);
    }

    #[test]
    fn only_shared_icons_are_retried_a_few_times() {
        assert!(may_retry(".pdf:32", 1));
        assert!(may_retry("<folder>:32", 2));
        assert!(!may_retry(".pdf:32", 3));
        assert!(!may_retry(r"C:\a.exe:32", 1));
    }
}
