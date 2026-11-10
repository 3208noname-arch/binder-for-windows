//! サムネイルの順番待ち。保存済みを探す1本と、作る少数のスレッドに分ける。
//! 作る側が遅いNASで待たされても、保存済みの絵は出続ける。

use crate::thumb_store::{Lookup, ThumbId, ThumbStore};
use crate::thumbnail::{Made, ThumbKind, make_thumbnail};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};

/// 同時に作る数の上限。
const MAX_MAKERS: usize = 2;
/// この件数を書くたびに、保存の上限を確かめる。
const TRIM_EVERY: usize = 256;

#[derive(Clone, Debug)]
pub struct ThumbJob {
    pub key: String,
    pub path: PathBuf,
    pub side: u32,
    pub kind: ThumbKind,
    pub modified: i64,
    pub length: u64,
    /// 画面の外になっても取り消さない
    pub keep: bool,
}

#[derive(Debug)]
pub enum Thumb {
    Picture(Vec<u8>),
    /// 作れない形式。種類のアイコンのままにする
    Unavailable,
    /// ファイルに届かなかった。時間を置いて取り直す
    Later,
    Cancelled,
}

struct Running {
    cancel: Arc<AtomicBool>,
    keep: bool,
}

#[derive(Default)]
struct Lanes {
    lookup: VecDeque<ThumbJob>,
    make: VecDeque<ThumbJob>,
    running: HashMap<String, Running>,
    stop: bool,
}

impl Lanes {
    /// `wanted` に無い順番待ちを外して返す。作っている途中のものには取り消しの印を付ける。
    fn keep_only(&mut self, wanted: &HashSet<String>) -> Vec<String> {
        let mut cancelled = Vec::new();
        let mut filter = |lane: &mut VecDeque<ThumbJob>| {
            lane.retain(|job| {
                let stays = job.keep || wanted.contains(&job.key);
                if !stays {
                    cancelled.push(job.key.clone());
                }
                stays
            });
        };
        filter(&mut self.lookup);
        filter(&mut self.make);
        for (key, running) in &self.running {
            if !running.keep && !wanted.contains(key) {
                running.cancel.store(true, Ordering::Relaxed);
            }
        }
        cancelled
    }
}

struct Shared {
    lanes: Mutex<Lanes>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Lanes> {
        self.lanes.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait<'a>(&self, guard: MutexGuard<'a, Lanes>) -> MutexGuard<'a, Lanes> {
        self.changed.wait(guard).unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct ThumbPool {
    shared: Arc<Shared>,
}

type Deliver = dyn Fn(String, Thumb) + Send + Sync;

fn id(job: &ThumbJob) -> ThumbId<'_> {
    ThumbId { path: &job.path, modified: job.modified, length: job.length, side: job.side }
}

fn run_lookup(shared: &Shared, store: Option<&ThumbStore>, deliver: &Deliver) {
    if let Some(store) = store {
        store.trim();
    }
    loop {
        let job = {
            let mut lanes = shared.lock();
            loop {
                if lanes.stop {
                    return;
                }
                if let Some(job) = lanes.lookup.pop_front() {
                    break job;
                }
                lanes = shared.wait(lanes);
            }
        };
        match store.map_or(Lookup::Missing, |store| store.get(&id(&job))) {
            Lookup::Picture(bytes) => deliver(job.key, Thumb::Picture(bytes)),
            Lookup::Failed => deliver(job.key, Thumb::Unavailable),
            Lookup::Missing => {
                let mut lanes = shared.lock();
                lanes.make.push_back(job);
                shared.changed.notify_all();
            }
        }
    }
}

fn run_maker(shared: &Shared, store: Option<&ThumbStore>, written: &AtomicUsize, deliver: &Deliver) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
    loop {
        let (job, cancel) = {
            let mut lanes = shared.lock();
            loop {
                if lanes.stop {
                    return;
                }
                if let Some(job) = lanes.make.pop_front() {
                    let cancel = Arc::new(AtomicBool::new(false));
                    lanes.running.insert(job.key.clone(), Running { cancel: cancel.clone(), keep: job.keep });
                    break (job, cancel);
                }
                lanes = shared.wait(lanes);
            }
        };
        let made = make_thumbnail(&job.path, job.side, job.kind, &|| cancel.load(Ordering::Relaxed));
        shared.lock().running.remove(&job.key);
        let thumb = match made {
            Made::Picture(bytes) => {
                if let Some(store) = store {
                    store.put(&id(&job), &bytes);
                    if (written.fetch_add(1, Ordering::Relaxed) + 1).is_multiple_of(TRIM_EVERY) {
                        store.trim();
                    }
                }
                Thumb::Picture(bytes)
            }
            Made::Unsupported => {
                if let Some(store) = store {
                    store.put_failed(&id(&job));
                }
                Thumb::Unavailable
            }
            Made::Unreachable => Thumb::Later,
            Made::Cancelled => Thumb::Cancelled,
        };
        deliver(job.key, thumb);
    }
}

impl ThumbPool {
    /// `store` が `None` なら保存せずに動く。`deliver` は裏のスレッドから呼ばれる。
    pub fn start(store: Option<ThumbStore>, deliver: impl Fn(String, Thumb) + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared { lanes: Mutex::default(), changed: Condvar::new() });
        let store = Arc::new(store);
        let deliver: Arc<Deliver> = Arc::new(deliver);
        let written = Arc::new(AtomicUsize::new(0));
        let cores = std::thread::available_parallelism().map_or(1, |count| count.get());
        let makers = (cores / 2).clamp(1, MAX_MAKERS);
        {
            let (shared, store, deliver) = (shared.clone(), store.clone(), deliver.clone());
            let _ = std::thread::Builder::new().name("thumb-lookup".into()).spawn(move || run_lookup(&shared, store.as_ref().as_ref(), deliver.as_ref()));
        }
        for number in 0..makers {
            let (shared, store, deliver, written) = (shared.clone(), store.clone(), deliver.clone(), written.clone());
            let _ = std::thread::Builder::new()
                .name(format!("thumb-make-{number}"))
                .spawn(move || run_maker(&shared, store.as_ref().as_ref(), &written, deliver.as_ref()));
        }
        Self { shared }
    }

    pub fn request(&self, job: ThumbJob) {
        self.shared.lock().lookup.push_back(job);
        self.shared.changed.notify_all();
    }

    /// 今見えているものだけを残す。順番待ちから外したキーを返す。
    pub fn keep_only(&self, wanted: &HashSet<String>) -> Vec<String> {
        self.shared.lock().keep_only(wanted)
    }
}

impl Drop for ThumbPool {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(key: &str, keep: bool) -> ThumbJob {
        ThumbJob { key: key.into(), path: PathBuf::from(key), side: 128, kind: ThumbKind::Image, modified: 1, length: 1, keep }
    }

    fn wanted(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn jobs_off_screen_leave_both_queues_but_kept_jobs_stay() {
        let mut lanes = Lanes::default();
        lanes.lookup.extend([job("visible", false), job("gone", false)]);
        lanes.make.extend([job("gone2", false), job("quick-look", true)]);
        let mut cancelled = lanes.keep_only(&wanted(&["visible"]));
        cancelled.sort();
        assert_eq!(cancelled, vec!["gone", "gone2"]);
        assert_eq!(lanes.lookup.len(), 1);
        assert_eq!(lanes.make[0].key, "quick-look");
    }

    #[test]
    fn a_running_job_off_screen_is_told_to_stop() {
        let mut lanes = Lanes::default();
        let (gone, visible, kept) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        lanes.running.insert("gone".into(), Running { cancel: gone.clone(), keep: false });
        lanes.running.insert("visible".into(), Running { cancel: visible.clone(), keep: false });
        lanes.running.insert("kept".into(), Running { cancel: kept.clone(), keep: true });
        lanes.keep_only(&wanted(&["visible"]));
        assert!(gone.load(Ordering::Relaxed));
        assert!(!visible.load(Ordering::Relaxed));
        assert!(!kept.load(Ordering::Relaxed));
    }
}
