use filer_core::Entry;
use filer_shell::{init_thread, load_icon};
use futures::StreamExt;
use gpui::{Context, Image, ImageFormat, Task};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

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

struct Request {
    key: String,
    path: PathBuf,
    size: u32,
    thumbnail: bool,
}

enum Loaded {
    Done(Option<Vec<u8>>),
    /// 順番待ちが溜まって捨てた要求。見えていれば次の描画で取り直す
    Dropped,
}

/// 取得中と失敗は `None` で覚える。1件の失敗で同じ種類の全部が仮の図形にならないよう、
/// 種類で共有する絵だけは別のファイルで数回取り直す。
pub struct IconCache {
    icons: HashMap<String, Option<Arc<Image>>>,
    failures: HashMap<String, u8>,
    requests: Sender<Request>,
    _receiver: Task<()>,
}

fn may_retry(key: &str, failures: u8) -> bool {
    let shared = key.starts_with('.') || key.starts_with("<folder>");
    shared && failures < MAX_RETRIES
}

impl IconCache {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (requests, queue) = channel::<Request>();
        let (done, mut results) = futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            init_thread();
            let mut pending = Vec::new();
            while let Ok(first) = queue.recv() {
                pending.push(first);
                // 新しい要求ほど今見えている行なので、後ろから取る
                while let Some(request) = pending.pop() {
                    let png = load_icon(&request.path, request.size, request.thumbnail);
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
        Self { icons: HashMap::new(), failures: HashMap::new(), requests, _receiver: receiver }
    }

    fn receive(&mut self, key: String, loaded: Loaded) {
        match loaded {
            Loaded::Done(Some(bytes)) => {
                self.icons.insert(key, Some(Arc::new(Image::from_bytes(ImageFormat::Png, bytes))));
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
        }
    }

    /// 持っていれば返す。無ければ裏で取りに行き、届いたら画面を更新する。
    pub fn get(&mut self, folder: &Path, entry: &Entry, size: u32) -> Option<Arc<Image>> {
        let extension = entry.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        let e = extension.as_str();
        let path = folder.join(entry.os_name());
        let thumbnail = size >= LARGE && (IMAGES.contains(&e) || VIDEOS.contains(&e));
        let key = match (entry.is_dir, thumbnail || PER_FILE.contains(&e)) {
            (true, _) => format!("<folder>:{size}"),
            (false, true) => format!("{}:{size}", path.display()),
            (false, false) => format!(".{e}:{size}"),
        };
        if let Some(icon) = self.icons.get(&key) {
            return icon.clone();
        }
        self.icons.insert(key.clone(), None);
        let _ = self.requests.send(Request { key, path, size, thumbnail });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::may_retry;

    #[test]
    fn only_shared_icons_are_retried_a_few_times() {
        assert!(may_retry(".pdf:32", 1));
        assert!(may_retry("<folder>:32", 2));
        assert!(!may_retry(".pdf:32", 3));
        assert!(!may_retry(r"C:\a.exe:32", 1));
    }
}
