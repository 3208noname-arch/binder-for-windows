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

/// 1件ごとに違う絵を持つ種類。これ以外は拡張子で共有する。
const PER_FILE: &[&str] = &["exe", "lnk", "ico", "url"];
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tif", "tiff", "heic"];
const VIDEOS: &[&str] = &["mp4", "mov", "mkv", "avi", "webm"];

struct Request {
    key: String,
    path: PathBuf,
    size: u32,
    thumbnail: bool,
}

/// 裏のスレッドで取ったアイコンを持つ。取れなかったものは `None` で覚え、再度は取らない。
pub struct IconCache {
    icons: HashMap<String, Option<Arc<Image>>>,
    requests: Sender<Request>,
    _receiver: Task<()>,
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
                    if done.unbounded_send((request.key, png)).is_err() {
                        return;
                    }
                    pending.extend(queue.try_iter());
                }
            }
        });
        let receiver = cx.spawn(async move |this, cx| {
            while let Some((key, png)) = results.next().await {
                let image = png.map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)));
                let stored = this.update(cx, |cache, cx| {
                    cache.icons.insert(key, image);
                    cx.notify();
                });
                if stored.is_err() {
                    break;
                }
            }
        });
        Self { icons: HashMap::new(), requests, _receiver: receiver }
    }

    /// 持っていれば返す。無ければ裏で取りに行き、届いたら画面を更新する。
    pub fn get(&mut self, folder: &Path, entry: &Entry, size: u32) -> Option<Arc<Image>> {
        let extension = entry.name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        let e = extension.as_str();
        let path = folder.join(entry.os_name());
        let thumbnail = size == LARGE && (IMAGES.contains(&e) || VIDEOS.contains(&e));
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
