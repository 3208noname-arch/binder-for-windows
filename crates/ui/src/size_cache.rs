use filer_shell::{FolderSize, folder_size};
use futures::StreamExt;
use gpui::{Context, Task};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

const MAX_PENDING: usize = 256;

struct Request {
    path: PathBuf,
    generation: u64,
}

/// フォルダの合計サイズを裏で計算して持つ。見えている行の分だけ要求される。
pub struct SizeCache {
    sizes: HashMap<PathBuf, Option<u64>>,
    requests: Sender<Request>,
    /// フォルダを移るたびに増やす。古い要求は計算せずに捨てる
    generation: Arc<AtomicU64>,
    /// 計算中の仕事を止める旗
    running: Arc<Mutex<Arc<AtomicBool>>>,
    _receiver: Task<()>,
}

impl SizeCache {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (requests, queue) = channel::<Request>();
        let (done, mut results) = futures::channel::mpsc::unbounded();
        let generation = Arc::new(AtomicU64::new(0));
        let running = Arc::new(Mutex::new(Arc::new(AtomicBool::new(false))));
        let (current, flag) = (generation.clone(), running.clone());
        std::thread::spawn(move || {
            let mut pending = Vec::new();
            while let Ok(first) = queue.recv() {
                pending.push(first);
                while let Some(request) = pending.pop() {
                    let size = if request.generation == current.load(Ordering::Relaxed) {
                        let cancel = flag.lock().unwrap().clone();
                        folder_size(&request.path, &cancel)
                    } else {
                        FolderSize::Cancelled
                    };
                    if done.unbounded_send((request.path, size)).is_err() {
                        return;
                    }
                    pending.extend(queue.try_iter());
                    let excess = pending.len().saturating_sub(MAX_PENDING);
                    for old in pending.drain(..excess) {
                        let _ = done.unbounded_send((old.path, FolderSize::Cancelled));
                    }
                }
            }
        });
        let receiver = cx.spawn(async move |this, cx| {
            while let Some((path, size)) = results.next().await {
                let stored = this.update(cx, |cache, cx| {
                    match size {
                        FolderSize::Bytes(size) => {
                            cache.sizes.insert(path, Some(size));
                        }
                        // 止めた分は忘れる。また見えた時に計算し直す
                        FolderSize::Cancelled => {
                            cache.sizes.remove(&path);
                        }
                        // 読めないフォルダは `None` のまま覚え、計算し直さない
                        FolderSize::Unreadable => {}
                    }
                    cx.notify();
                });
                if stored.is_err() {
                    break;
                }
            }
        });
        Self { sizes: HashMap::new(), requests, generation, running, _receiver: receiver }
    }

    /// 分かっていれば返す。無ければ裏で計算を始める。
    pub fn get(&mut self, path: &Path) -> Option<u64> {
        if let Some(size) = self.sizes.get(path) {
            return *size;
        }
        self.sizes.insert(path.to_path_buf(), None);
        let generation = self.generation.load(Ordering::Relaxed);
        let _ = self.requests.send(Request { path: path.to_path_buf(), generation });
        None
    }

    /// 分かっている時だけ返す。計算は始めない。
    pub fn peek(&self, path: &Path) -> Option<u64> {
        self.sizes.get(path).copied().flatten()
    }

    /// 今の計算と順番待ちを捨てる。別のフォルダへ移った時に呼ぶ。
    pub fn cancel_running(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        let mut running = self.running.lock().unwrap();
        running.store(true, Ordering::Relaxed);
        *running = Arc::new(AtomicBool::new(false));
    }
}
