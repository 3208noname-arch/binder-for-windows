use std::path::{Path, PathBuf};

/// 戻ると進むの履歴。
pub struct History {
    current: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl History {
    pub fn new(start: PathBuf) -> Self {
        Self { current: start, back: Vec::new(), forward: Vec::new() }
    }
    pub fn current(&self) -> &Path {
        &self.current
    }
    /// 新しい場所へ移る。同じ場所なら履歴を増やさない。
    pub fn visit(&mut self, path: PathBuf) {
        if path == self.current {
            return;
        }
        self.back.push(std::mem::replace(&mut self.current, path));
        self.forward.clear();
    }
    pub fn go_back(&mut self) -> Option<&Path> {
        let previous = self.back.pop()?;
        self.forward.push(std::mem::replace(&mut self.current, previous));
        Some(&self.current)
    }
    pub fn go_forward(&mut self) -> Option<&Path> {
        let next = self.forward.pop()?;
        self.back.push(std::mem::replace(&mut self.current, next));
        Some(&self.current)
    }
    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }
    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        s.into()
    }

    #[test]
    fn back_and_forward_move_between_visits() {
        let mut h = History::new(p("a"));
        h.visit(p("b"));
        assert_eq!(h.go_back(), Some(Path::new("a")));
        assert!(!h.can_go_back());
        assert_eq!(h.go_forward(), Some(Path::new("b")));
        assert!(!h.can_go_forward());
    }

    #[test]
    fn visiting_after_back_clears_forward() {
        let mut h = History::new(p("a"));
        h.visit(p("b"));
        h.go_back();
        h.visit(p("c"));
        assert!(!h.can_go_forward());
        assert_eq!(h.current(), Path::new("c"));
    }

    #[test]
    fn visiting_same_place_adds_nothing() {
        let mut h = History::new(p("a"));
        h.visit(p("a"));
        assert!(!h.can_go_back());
    }

    #[test]
    fn empty_history_returns_none() {
        let mut h = History::new(p("a"));
        assert_eq!(h.go_back(), None);
        assert_eq!(h.go_forward(), None);
    }
}
