use filer_core::{Entry, SortKey, SortOrder, insert_sorted, sort_entries};
use filer_shell::{ListEvent, compare_names};
use std::path::PathBuf;

/// 開いているフォルダの一覧の状態。画面から切り離して試験する。
pub struct ListState {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub order: SortOrder,
    pub error: Option<String>,
    pub generation: u64,
}

impl ListState {
    pub fn new(path: PathBuf) -> Self {
        let order = SortOrder { key: SortKey::Name, descending: false };
        Self { path, entries: Vec::new(), order, error: None, generation: 0 }
    }

    /// 別のフォルダへ移る。返した世代より古い列挙の結果は `apply` で捨てる。
    pub fn open(&mut self, path: PathBuf) -> u64 {
        self.path = path;
        self.entries.clear();
        self.error = None;
        self.generation += 1;
        self.generation
    }

    pub fn apply(&mut self, generation: u64, event: ListEvent) {
        if generation != self.generation {
            return;
        }
        match event {
            ListEvent::Batch(batch) => insert_sorted(&mut self.entries, batch, self.order, compare_names),
            ListEvent::Done => {}
            ListEvent::Failed(message) => self.error = Some(message),
        }
    }

    pub fn set_order(&mut self, order: SortOrder) {
        self.order = order;
        sort_entries(&mut self.entries, order, compare_names);
    }
}

/// 溜まった列挙の結果を1つのバッチにまとめる。併合の回数を減らし、描画を止めないため。
pub fn coalesce(events: Vec<ListEvent>) -> Vec<ListEvent> {
    let mut joined = Vec::new();
    let mut others = Vec::new();
    for event in events {
        match event {
            ListEvent::Batch(batch) => joined.extend(batch),
            other => others.push(other),
        }
    }
    let mut result = Vec::with_capacity(others.len() + 1);
    if !joined.is_empty() {
        result.push(ListEvent::Batch(joined));
    }
    result.extend(others);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesce_joins_batches_before_final_event() {
        let events = vec![
            ListEvent::Batch(vec![file("a")]),
            ListEvent::Batch(vec![file("b")]),
            ListEvent::Done,
        ];
        let result = coalesce(events);
        assert_eq!(result.len(), 2);
        match &result[0] {
            ListEvent::Batch(b) => assert_eq!(b.len(), 2),
            _ => panic!("batch first"),
        }
        assert!(matches!(result[1], ListEvent::Done));
    }

    fn file(name: &str) -> Entry {
        Entry { name: name.into(), is_dir: false, size: 0, modified: 0 }
    }
    fn names(state: &ListState) -> Vec<&str> {
        state.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn open_clears_previous_folder() {
        let mut s = ListState::new("a".into());
        let g = s.open("a".into());
        s.apply(g, ListEvent::Batch(vec![file("x")]));
        s.apply(g, ListEvent::Failed("no".into()));
        s.open("b".into());
        assert!(s.entries.is_empty());
        assert_eq!(s.error, None);
    }

    #[test]
    fn stale_generation_is_ignored() {
        let mut s = ListState::new("a".into());
        let old = s.open("a".into());
        let new = s.open("b".into());
        s.apply(old, ListEvent::Batch(vec![file("old")]));
        s.apply(new, ListEvent::Batch(vec![file("new")]));
        assert_eq!(names(&s), ["new"]);
    }

    #[test]
    fn failed_event_sets_error() {
        let mut s = ListState::new("a".into());
        let g = s.open("a".into());
        s.apply(g, ListEvent::Failed("アクセスが拒否されました".into()));
        assert_eq!(s.error.as_deref(), Some("アクセスが拒否されました"));
    }

    #[test]
    fn batch_during_sort_stays_ordered() {
        let mut s = ListState::new("a".into());
        let g = s.open("a".into());
        s.apply(g, ListEvent::Batch(vec![file("a2"), file("a10")]));
        s.set_order(SortOrder { key: SortKey::Name, descending: true });
        s.apply(g, ListEvent::Batch(vec![file("a3")]));
        assert_eq!(names(&s), ["a10", "a3", "a2"]);
    }
}
