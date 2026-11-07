use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 選択中の項目。`ordered` は画面に並んでいる順のパス。
#[derive(Default)]
pub struct Selection {
    items: HashSet<PathBuf>,
    anchor: Option<PathBuf>,
    cursor: Option<PathBuf>,
    soft: bool,
}

impl Selection {
    pub fn contains(&self, path: &Path) -> bool {
        self.items.contains(path)
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn paths(&self) -> Vec<PathBuf> {
        self.items.iter().cloned().collect()
    }
    /// 選択中のフォルダの中にある項目を除いた一覧。ファイル操作に使う。
    pub fn top_level(&self) -> Vec<PathBuf> {
        let inside_selected = |path: &&PathBuf| path.ancestors().skip(1).any(|parent| self.items.contains(parent));
        self.items.iter().filter(|path| !inside_selected(path)).cloned().collect()
    }
    /// 範囲選択で選んだ状態か。次に選択を変えるかキーを押すまで、薄い色で示す。
    pub fn is_soft(&self) -> bool {
        self.soft
    }
    pub fn soften(&mut self) {
        self.soft = true;
    }
    pub fn harden(&mut self) {
        self.soft = false;
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    /// 普通のクリック。その項目だけを選ぶ。
    pub fn select_only(&mut self, path: PathBuf) {
        self.soft = false;
        self.items.clear();
        self.items.insert(path.clone());
        self.anchor = Some(path.clone());
        self.cursor = Some(path);
    }
    /// Ctrl+クリック。
    pub fn toggle(&mut self, path: PathBuf) {
        self.soft = false;
        if !self.items.remove(&path) {
            self.items.insert(path.clone());
        }
        self.anchor = Some(path.clone());
        self.cursor = Some(path);
    }
    /// Shift+クリック。基準の項目からそこまでを選ぶ。
    pub fn extend_to(&mut self, path: &Path, ordered: &[PathBuf]) {
        let anchor = self.anchor.clone().unwrap_or_else(|| path.to_path_buf());
        let position = |target: &Path| ordered.iter().position(|item| item == target);
        let (Some(from), Some(to)) = (position(&anchor), position(path)) else { return };
        self.soft = false;
        self.items = ordered[from.min(to)..=from.max(to)].iter().cloned().collect();
        self.anchor = Some(anchor);
        self.cursor = Some(path.to_path_buf());
    }
    pub fn select_all(&mut self, ordered: &[PathBuf]) {
        self.soft = false;
        self.items = ordered.iter().cloned().collect();
    }
    /// 矢印キー。`delta` 行動かす。`extend` はShiftを押している時。動いた先を返す。
    pub fn move_cursor(&mut self, delta: isize, extend: bool, ordered: &[PathBuf]) -> Option<PathBuf> {
        let last = ordered.len().checked_sub(1)? as isize;
        let current = self.cursor.as_ref().and_then(|cursor| ordered.iter().position(|item| item == cursor));
        let target = match current {
            Some(index) => (index as isize + delta).clamp(0, last),
            None if delta > 0 => 0,
            None => last,
        };
        let path = ordered[target as usize].clone();
        if extend {
            self.extend_to(&path, ordered);
        } else {
            self.select_only(path.clone());
        }
        Some(path)
    }
    /// 画面から消えた項目を選択から外す。
    pub fn retain_existing(&mut self, ordered: &[PathBuf]) {
        let existing: HashSet<&PathBuf> = ordered.iter().collect();
        self.items.retain(|item| existing.contains(item));
        self.anchor.take_if(|path| !existing.contains(path));
        self.cursor.take_if(|path| !existing.contains(path));
        self.soft &= !self.items.is_empty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Vec<PathBuf> {
        ["a", "b", "c", "d"].map(PathBuf::from).to_vec()
    }
    fn sorted(s: &Selection) -> Vec<String> {
        let mut paths: Vec<_> = s.paths().iter().map(|p| p.display().to_string()).collect();
        paths.sort();
        paths
    }

    #[test]
    fn every_change_ends_the_soft_look() {
        let changes: [fn(&mut Selection); 6] = [
            |s| s.select_only(PathBuf::from("a")),
            |s| s.toggle(PathBuf::from("a")),
            |s| s.extend_to(Path::new("b"), &list()),
            |s| s.select_all(&list()),
            |s| s.clear(),
            |s| {
                s.move_cursor(1, false, &list());
            },
        ];
        for change in changes {
            let mut s = Selection::default();
            s.select_all(&list());
            s.soften();
            assert!(s.is_soft());
            change(&mut s);
            assert!(!s.is_soft());
        }
    }

    #[test]
    fn soft_look_ends_when_nothing_is_left() {
        let mut s = Selection::default();
        s.select_all(&list());
        s.soften();
        s.retain_existing(&[PathBuf::from("a")]);
        assert!(s.is_soft());
        s.retain_existing(&[]);
        assert!(!s.is_soft());
    }

    #[test]
    fn soft_look_is_not_set_by_selecting() {
        let mut s = Selection::default();
        s.select_only(PathBuf::from("a"));
        assert!(!s.is_soft());
        s.soften();
        s.harden();
        assert!(!s.is_soft());
    }

    #[test]
    fn plain_click_replaces_selection() {
        let mut s = Selection::default();
        s.select_only("a".into());
        s.select_only("b".into());
        assert_eq!(sorted(&s), ["b"]);
    }

    #[test]
    fn ctrl_click_adds_and_removes() {
        let mut s = Selection::default();
        s.select_only("a".into());
        s.toggle("c".into());
        assert_eq!(sorted(&s), ["a", "c"]);
        s.toggle("a".into());
        assert_eq!(sorted(&s), ["c"]);
    }

    #[test]
    fn shift_click_selects_range_from_anchor() {
        let mut s = Selection::default();
        s.select_only("b".into());
        s.extend_to(Path::new("d"), &list());
        assert_eq!(sorted(&s), ["b", "c", "d"]);
        s.extend_to(Path::new("a"), &list());
        assert_eq!(sorted(&s), ["a", "b"]);
    }

    #[test]
    fn select_all_takes_everything() {
        let mut s = Selection::default();
        s.select_all(&list());
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn arrows_move_and_stop_at_edges() {
        let mut s = Selection::default();
        assert_eq!(s.move_cursor(1, false, &list()), Some("a".into()));
        assert_eq!(s.move_cursor(1, false, &list()), Some("b".into()));
        assert_eq!(sorted(&s), ["b"]);
        assert_eq!(s.move_cursor(-5, false, &list()), Some("a".into()));
        assert_eq!(s.move_cursor(9, false, &list()), Some("d".into()));
    }

    #[test]
    fn shift_arrow_extends_selection() {
        let mut s = Selection::default();
        s.select_only("b".into());
        s.move_cursor(1, true, &list());
        assert_eq!(sorted(&s), ["b", "c"]);
    }

    #[test]
    fn top_level_drops_items_inside_selected_folders() {
        let mut s = Selection::default();
        s.select_all(&["a".into(), Path::new("a").join("b"), "c".into()]);
        let mut top: Vec<_> = s.top_level().iter().map(|p| p.display().to_string()).collect();
        top.sort();
        assert_eq!(top, ["a", "c"]);
    }

    #[test]
    fn missing_items_leave_selection() {
        let mut s = Selection::default();
        s.select_all(&list());
        s.retain_existing(&["a".into()]);
        assert_eq!(sorted(&s), ["a"]);
    }
}
