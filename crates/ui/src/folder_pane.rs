use crate::list_state::{ListState, coalesce};
use filer_core::{Entry, SortKey, SortOrder};
use filer_shell::{ListJob, start_list};
use futures::StreamExt;
use gpui::{Context, Task};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MERGE_INTERVAL: Duration = Duration::from_millis(8);

/// 1つのフォルダの一覧。3つの表示と入れ子で共有する。
pub struct FolderPane {
    pub state: ListState,
    job: Option<ListJob>,
    loader: Option<Task<()>>,
    measure: Option<Measure>,
}

struct Measure {
    start: Instant,
    first_row_seen: bool,
    reopened: bool,
}

/// 測定用に順番を崩す。Rustの並べ替えは整列済みの入力で速いので、そのままでは測れない。
fn shuffle(entries: &mut [Entry]) {
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    for i in (1..entries.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        entries.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

impl FolderPane {
    pub fn new(path: PathBuf, order: SortOrder, cx: &mut Context<Self>) -> Self {
        let mut state = ListState::new(path.clone());
        state.order = order;
        let mut pane = Self { state, job: None, loader: None, measure: None };
        pane.open(path, cx);
        pane
    }

    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let generation = self.state.open(path.clone());
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        self.job = Some(start_list(path, move |event| {
            let _ = tx.unbounded_send(event);
        }));
        self.loader = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut events = vec![first];
                while let Ok(event) = rx.try_recv() {
                    events.push(event);
                }
                let applied = this.update(cx, |pane, cx| {
                    for event in coalesce(events) {
                        pane.state.apply(generation, event);
                    }
                    pane.note_first_row();
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
                // 描画の順番を譲り、その間に届いた分をまとめて併合する
                cx.background_executor().timer(MERGE_INTERVAL).await;
            }
            let _ = this.update(cx, |pane, cx| pane.finish_measure(cx));
        }));
        cx.notify();
    }

    pub fn set_order(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        self.state.set_order(order);
        cx.notify();
    }

    /// `--measure` の時だけ呼ぶ。測定の開始時刻を覚える。
    pub fn measure_from(&mut self, start: Instant) {
        self.measure = Some(Measure { start, first_row_seen: false, reopened: false });
    }

    fn note_first_row(&mut self) {
        let Some(measure) = &mut self.measure else { return };
        if !measure.first_row_seen && !self.state.entries.is_empty() {
            measure.first_row_seen = true;
            let label = if measure.reopened { "reopen_first_row_ms" } else { "first_row_ms" };
            println!("{label}={}", measure.start.elapsed().as_millis());
        }
    }

    fn finish_measure(&mut self, cx: &mut Context<Self>) {
        let Some(measure) = &mut self.measure else { return };
        println!("all_entries_ms={} count={}", measure.start.elapsed().as_millis(), self.state.entries.len());
        if measure.reopened {
            cx.quit();
            return;
        }
        for key in [SortKey::Modified, SortKey::Size, SortKey::Kind, SortKey::Name] {
            shuffle(&mut self.state.entries);
            let started = Instant::now();
            self.state.set_order(SortOrder { key, descending: false });
            println!("sort_{key:?}_ms={}", started.elapsed().as_millis());
        }
        // 2回目は、起動の時間を含まないフォルダ移動の時間を測る
        *measure = Measure { start: Instant::now(), first_row_seen: false, reopened: true };
        let path = self.state.path.clone();
        self.open(path, cx);
    }
}
