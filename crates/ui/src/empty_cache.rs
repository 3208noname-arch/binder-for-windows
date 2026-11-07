use futures::StreamExt;
use gpui::{Context, Task};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};

/// フォルダが空かを裏で調べて持つ。見えている行の分だけ要求される。
/// 中身が変わるとフォルダの更新日時も変わるので、日時が違えば調べ直す。
pub struct EmptyCache {
    known: HashMap<PathBuf, (i64, bool)>,
    requests: Sender<(PathBuf, i64)>,
    _receiver: Task<()>,
}

impl EmptyCache {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (requests, queue) = channel::<(PathBuf, i64)>();
        let (done, mut results) = futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            let mut pending = Vec::new();
            while let Ok(first) = queue.recv() {
                pending.push(first);
                // 新しい要求から調べる。スクロールで見えなくなった行は後回しになる
                while let Some((path, stamp)) = pending.pop() {
                    let empty = std::fs::read_dir(&path).is_ok_and(|mut items| items.next().is_none());
                    if done.unbounded_send((path, stamp, empty)).is_err() {
                        return;
                    }
                    pending.extend(queue.try_iter());
                }
            }
        });
        let receiver = cx.spawn(async move |this, cx| {
            while let Some((path, stamp, empty)) = results.next().await {
                let stored = this.update(cx, |cache, cx| {
                    cache.known.insert(path, (stamp, empty));
                    if empty {
                        cx.notify();
                    }
                });
                if stored.is_err() {
                    break;
                }
            }
        });
        Self { known: HashMap::new(), requests, _receiver: receiver }
    }

    /// 空と分かっていれば真。調べていなければ裏で調べ始める。
    pub fn get(&mut self, path: &Path, stamp: i64) -> bool {
        match self.known.get(path) {
            Some((known, empty)) if *known == stamp => *empty,
            _ => {
                // 結果が届くまでの間に同じ要求を重ねないよう、先に覚えておく
                self.known.insert(path.to_path_buf(), (stamp, false));
                let _ = self.requests.send((path.to_path_buf(), stamp));
                false
            }
        }
    }
}
