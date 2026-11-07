//! タブの並びと、次回の起動へ残す内容。画面の部品には依存しない。

use crate::ViewMode;
use serde::{Deserialize, Serialize};
use std::mem::{replace, take};

/// 選んでいるタブ以外の並び。選んでいるタブは持ち主が別に持つので、タブは常に1枚以上ある。
///
/// `index` は全タブの中での、選んでいるタブの位置。
pub struct TabList<T> {
    others: Vec<T>,
    index: usize,
}

impl<T> Default for TabList<T> {
    fn default() -> Self {
        Self { others: Vec::new(), index: 0 }
    }
}

impl<T> TabList<T> {
    pub fn len(&self) -> usize {
        self.others.len() + 1
    }

    /// タブは必ず1枚以上あるので、常に偽。
    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// 左から順に、選んでいるタブも含めて辿る。
    pub fn iter<'a>(&'a self, current: &'a T) -> impl Iterator<Item = &'a T> {
        let (left, right) = self.others.split_at(self.index);
        left.iter().chain(std::iter::once(current)).chain(right)
    }

    pub fn position(&self, current: &T, found: impl FnMut(&T) -> bool) -> Option<usize> {
        self.iter(current).position(found)
    }

    /// `index` のタブを選ぶ。範囲外と、すでに選んでいるタブでは何もしない。
    pub fn select(&mut self, index: usize, current: &mut T) -> bool {
        if index >= self.len() || index == self.index {
            return false;
        }
        let picked = self.others.remove(self.slot(index));
        let previous = replace(current, picked);
        self.others.insert(if index < self.index { self.index - 1 } else { self.index }, previous);
        self.index = index;
        true
    }

    /// 隣のタブの位置。端の次は反対の端へ戻る。
    pub fn neighbor(&self, forward: bool) -> usize {
        (self.index + if forward { 1 } else { self.len() - 1 }) % self.len()
    }

    /// 選んでいるタブの右へ足して、選ぶ。
    pub fn open_after(&mut self, tab: T, current: &mut T) {
        self.others.insert(self.index, replace(current, tab));
        self.index += 1;
    }

    /// 選んでいるタブの右へ、裏のまま足す。
    pub fn insert_after(&mut self, tab: T) {
        self.others.insert(self.index, tab);
    }

    /// 選んでいるタブの左へ、裏のまま足す。
    pub fn insert_before(&mut self, tab: T) {
        self.others.insert(self.index, tab);
        self.index += 1;
    }

    /// `index` のタブを閉じて返す。選んでいたタブなら右隣、無ければ左隣を選ぶ。
    /// 最後の1枚は閉じない。
    pub fn close(&mut self, index: usize, current: &mut T) -> Option<T> {
        if index >= self.len() || self.len() == 1 {
            return None;
        }
        if index != self.index {
            let closed = self.others.remove(self.slot(index));
            if index < self.index {
                self.index -= 1;
            }
            return Some(closed);
        }
        let neighbor = if self.index < self.others.len() {
            self.others.remove(self.index)
        } else {
            self.index -= 1;
            self.others.pop().expect("タブが2枚以上あるので右か左に隣がある")
        };
        Some(replace(current, neighbor))
    }

    /// 全タブの中の位置を、`others` の中の位置へ直す。
    fn slot(&self, index: usize) -> usize {
        if index < self.index { index } else { index - 1 }
    }
}

/// 次回の起動で開き直すタブ。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    pub path: String,
    pub mode: ViewMode,
}

/// ウィンドウのタブの並びと、選んでいたタブ。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabSession {
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

impl TabSession {
    /// 開くタブを、選んでいたタブとその左、右に分ける。空の保存値は `None`。
    /// 選択の位置が壊れていても、端のタブを選ぶ。
    pub fn split(mut self) -> Option<(Vec<SavedTab>, SavedTab, Vec<SavedTab>)> {
        if self.tabs.is_empty() {
            return None;
        }
        let active = self.active.min(self.tabs.len() - 1);
        let right = self.tabs.split_off(active + 1);
        let current = self.tabs.pop()?;
        Some((take(&mut self.tabs), current, right))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FolderView, History, Selection};
    use std::path::{Path, PathBuf};

    fn list(names: &[&'static str], index: usize) -> (TabList<&'static str>, &'static str) {
        let mut tabs = TabList::default();
        let mut current = names[0];
        for name in &names[1..] {
            tabs.open_after(*name, &mut current);
        }
        tabs.select(index, &mut current);
        (tabs, current)
    }

    fn order(tabs: &TabList<&'static str>, current: &'static str) -> Vec<&'static str> {
        tabs.iter(&current).copied().collect()
    }

    #[test]
    fn selecting_keeps_the_order() {
        let (mut tabs, mut current) = list(&["a", "b", "c", "d"], 1);
        for index in [3, 0, 2, 1] {
            assert!(tabs.select(index, &mut current));
            assert_eq!(order(&tabs, current), ["a", "b", "c", "d"]);
            assert_eq!(current, ["a", "b", "c", "d"][index]);
            assert_eq!(tabs.index(), index);
        }
    }

    #[test]
    fn selecting_the_same_tab_or_outside_does_nothing() {
        let (mut tabs, mut current) = list(&["a", "b"], 1);
        assert!(!tabs.select(1, &mut current));
        assert!(!tabs.select(2, &mut current));
        assert_eq!(current, "b");
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let (mut tabs, mut current) = list(&["a", "b", "c"], 2);
        tabs.select(tabs.neighbor(true), &mut current);
        assert_eq!(current, "a");
        tabs.select(tabs.neighbor(false), &mut current);
        assert_eq!(current, "c");
        tabs.select(tabs.neighbor(false), &mut current);
        assert_eq!(current, "b");
    }

    #[test]
    fn a_single_tab_has_nowhere_to_go() {
        let (tabs, _) = list(&["a"], 0);
        assert_eq!(tabs.neighbor(true), 0);
        assert_eq!(tabs.neighbor(false), 0);
    }

    #[test]
    fn opened_tabs_go_right_of_the_current_one() {
        let (mut tabs, mut current) = list(&["a", "b"], 0);
        tabs.open_after("x", &mut current);
        assert_eq!(order(&tabs, current), ["a", "x", "b"]);
        assert_eq!(current, "x");
    }

    #[test]
    fn background_tabs_do_not_change_the_selection() {
        let (mut tabs, mut current) = list(&["a", "b"], 0);
        tabs.insert_after("x");
        assert_eq!(order(&tabs, current), ["a", "x", "b"]);
        assert_eq!(current, "a");
        tabs.insert_before("y");
        assert_eq!(order(&tabs, current), ["y", "a", "x", "b"]);
        assert_eq!(tabs.index(), 1);
        tabs.select(tabs.neighbor(true), &mut current);
        assert_eq!(current, "x");
    }

    #[test]
    fn closing_the_current_tab_selects_the_right_neighbor() {
        let (mut tabs, mut current) = list(&["a", "b", "c"], 1);
        assert_eq!(tabs.close(1, &mut current), Some("b"));
        assert_eq!(current, "c");
        assert_eq!(order(&tabs, current), ["a", "c"]);
    }

    #[test]
    fn closing_the_rightmost_tab_selects_the_left_neighbor() {
        let (mut tabs, mut current) = list(&["a", "b", "c"], 2);
        assert_eq!(tabs.close(2, &mut current), Some("c"));
        assert_eq!(current, "b");
        assert_eq!(tabs.index(), 1);
    }

    #[test]
    fn closing_another_tab_keeps_the_selection() {
        let (mut tabs, mut current) = list(&["a", "b", "c"], 1);
        assert_eq!(tabs.close(0, &mut current), Some("a"));
        assert_eq!(current, "b");
        assert_eq!(tabs.index(), 0);
        assert_eq!(tabs.close(1, &mut current), Some("c"));
        assert_eq!(current, "b");
        assert_eq!(tabs.index(), 0);
    }

    #[test]
    fn the_last_tab_cannot_be_closed() {
        let (mut tabs, mut current) = list(&["a"], 0);
        assert_eq!(tabs.close(0, &mut current), None);
        assert_eq!(current, "a");
    }

    struct Page {
        history: History,
        view: FolderView,
        selection: Selection,
    }

    fn page(path: &str, mode: ViewMode) -> Page {
        Page { history: History::new(path.into()), view: FolderView { mode, ..FolderView::default() }, selection: Selection::default() }
    }

    #[test]
    fn tabs_keep_their_own_history_mode_and_selection() {
        let mut tabs = TabList::default();
        let mut current = page(r"C:\a", ViewMode::Columns);
        current.history.visit(r"C:\a\deep".into());
        current.selection.select_only(PathBuf::from(r"C:\a\deep\file"));
        tabs.open_after(page(r"D:\b", ViewMode::Icons), &mut current);
        current.history.visit(r"D:\b\other".into());
        current.history.go_back();
        tabs.select(0, &mut current);
        assert_eq!(current.history.current(), Path::new(r"C:\a\deep"));
        assert!(current.history.can_go_back());
        assert!(!current.history.can_go_forward());
        assert_eq!(current.view.mode, ViewMode::Columns);
        assert_eq!(current.selection.paths(), [PathBuf::from(r"C:\a\deep\file")]);
        tabs.select(1, &mut current);
        assert_eq!(current.history.current(), Path::new(r"D:\b"));
        assert!(current.history.can_go_forward());
        assert_eq!(current.view.mode, ViewMode::Icons);
        assert!(current.selection.is_empty());
    }

    fn saved(path: &str) -> SavedTab {
        SavedTab { path: path.into(), mode: ViewMode::List }
    }

    #[test]
    fn session_splits_around_the_selected_tab() {
        let session = TabSession { tabs: vec![saved("a"), saved("b"), saved("c")], active: 1 };
        let (left, current, right) = session.split().unwrap();
        assert_eq!((left, current, right), (vec![saved("a")], saved("b"), vec![saved("c")]));
    }

    #[test]
    fn broken_session_selection_falls_back_to_the_last_tab() {
        let session = TabSession { tabs: vec![saved("a"), saved("b")], active: 9 };
        let (left, current, right) = session.split().unwrap();
        assert_eq!((left, current, right), (vec![saved("a")], saved("b"), vec![]));
    }

    #[test]
    fn empty_session_restores_nothing() {
        assert_eq!(TabSession::default().split(), None);
    }
}
