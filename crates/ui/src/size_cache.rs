use filer_shell::folder_size;
use futures::StreamExt;
use gpui::{Context, Task};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

const MAX_PENDING: usize = 256;

/// フォルダの合計サイズを裏で計算して持つ。見えている行の分だけ要求される。
pub struct SizeCache {
    sizes: HashMap<PathBuf, Option<u64>>,
    requests: Sender<PathBuf>,
    /// 計算中の仕事を止める旗。フォルダを移った時に立てる
    current: Arc<Mutex<Arc<AtomicBool>>>,
    _receiver: Task<()>,
}

impl SizeCache {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (requests, queue) = channel::<PathBuf>();
        let (done, mut results) = futures::channel::mpsc::unbounded();
        let current = Arc::new(Mutex::new(Arc::new(AtomicBool::new(false))));
        let flag = current.clone();
        std::thread::spawn(move || {
            let mut pending = Vec::new();
            while let Ok(first) = queue.recv() {
                pending.push(first);
                while let Some(path) = pending.pop() {
                    let cancel = flag.lock().unwrap().clone();
                    let size = folder_size(&path, &cancel);
                    if done.unbounded_send((path, size)).is_err() {
                        return;
                    }
                    pending.extend(queue.try_iter());
                    let excess = pending.len().saturating_sub(MAX_PENDING);
                    for old in pending.drain(..excess) {
                        let _ = done.unbounded_send((old, None));
                    }
                }
            }
        });
        let receiver = cx.spawn(async move |this, cx| {
            while let Some((path, size)) = results.next().await {
                let stored = this.update(cx, |cache, cx| {
                    match size {
                        Some(size) => {
                            cache.sizes.insert(path, Some(size));
                        }
                        // 止めた分は忘れる。また見えた時に計算し直す
                        None => {
                            cache.sizes.remove(&path);
                        }
                    }
                    cx.notify();
                });
                if stored.is_err() {
                    break;
                }
            }
        });
        Self { sizes: HashMap::new(), requests, current, _receiver: receiver }
    }

    /// 分かっていれば返す。無ければ裏で計算を始める。
    pub fn get(&mut self, path: &Path) -> Option<u64> {
        if let Some(size) = self.sizes.get(path) {
            return *size;
        }
        self.sizes.insert(path.to_path_buf(), None);
        let _ = self.requests.send(path.to_path_buf());
        None
    }

    /// 今の計算を止める。別のフォルダへ移った時に呼ぶ。
    pub fn cancel_running(&mut self) {
        let mut current = self.current.lock().unwrap();
        current.store(true, Ordering::Relaxed);
        *current = Arc::new(AtomicBool::new(false));
    }
}
