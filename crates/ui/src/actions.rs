//! 選択、キー操作、ファイル操作など、画面の表示以外の動作。

use crate::browser::Browser;
use crate::list_tree::{Flattened, Row};
use crate::text_input::{InputEvent, TextInput};
use crate::theme;
use filer_core::{Entry, ViewMode};
use filer_shell::{
    Operation, Prompt, clipboard_files, drag_items, init_thread, inverse, open_path, remove_ime_caret, run_operation,
    set_clipboard_files, show_folder_menu, show_items_menu, unique_folder_name, valid_file_name,
};
use gpui::{
    ClickEvent, Context, Entity, ExternalPaths, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, Point, ScrollStrategy, Stateful, Subscription, Window, prelude::*, rgb,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 名前を編集している最中の状態。
pub struct Renaming {
    pub path: PathBuf,
    pub input: Entity<TextInput>,
    _subscription: Subscription,
}

/// 項目の上でボタンを押した位置。動かすとドラッグになる。
pub struct RowPress {
    start: Point<Pixels>,
    path: PathBuf,
}

/// 2つのパスが同じドライブ（または共有）にあるか。
fn same_volume(a: &Path, b: &Path) -> bool {
    a.components().next() == b.components().next()
}

/// 行の並びを、行番号とパスの組へ直す。
fn items_of(tree: &Flattened, lists: &HashMap<&Path, &[Entry]>) -> Vec<(usize, PathBuf)> {
    let items = tree.rows.iter().enumerate().filter_map(|(ix, row)| {
        let Row::Item(row) = row else { return None };
        let folder = tree.folders[row.folder as usize].as_path();
        Some((ix, folder.join(lists.get(folder)?.get(row.entry as usize)?.os_name())))
    });
    items.collect()
}

impl Browser {
    /// 今操作しているフォルダ。カラムでは操作中の列。
    pub fn current_folder(&self, cx: &Context<Self>) -> PathBuf {
        match self.view.mode {
            ViewMode::Columns => self.columns[self.active_column].pane.read(cx).state.path.clone(),
            _ => self.history.current().to_path_buf(),
        }
    }

    /// 画面に並んでいる順の項目。
    fn visible_items(&mut self, cx: &Context<Self>) -> Vec<(usize, PathBuf)> {
        match self.view.mode {
            ViewMode::List => {
                let tree = self.tree(cx);
                let root = self.pane.read(cx);
                let mut lists = HashMap::from([(root.state.path.as_path(), root.state.entries.as_slice())]);
                for (path, pane) in &self.expanded {
                    lists.insert(path.as_path(), pane.read(cx).state.entries.as_slice());
                }
                items_of(&tree, &lists)
            }
            ViewMode::Columns => {
                let tree = self.column_tree(self.active_column, cx);
                let state = &self.columns[self.active_column].pane.read(cx).state;
                items_of(&tree, &HashMap::from([(state.path.as_path(), state.entries.as_slice())]))
            }
            ViewMode::Icons => {
                let pane = self.pane.read(cx);
                let (_, tree) = Self::plain_tree(pane, self.view.group);
                let state = &pane.state;
                items_of(&tree, &HashMap::from([(state.path.as_path(), state.entries.as_slice())]))
            }
        }
    }

    fn ordered_paths(&mut self, cx: &Context<Self>) -> Vec<PathBuf> {
        self.visible_items(cx).into_iter().map(|(_, path)| path).collect()
    }

    /// クリックでの選択。Ctrlは追加、Shiftは範囲。ダブルクリックは開く。
    pub fn click_item(&mut self, path: PathBuf, is_dir: bool, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.modifiers();
        if event.click_count() == 2 && !modifiers.control && !modifiers.shift {
            self.open_item(path, is_dir, cx);
            return;
        }
        if modifiers.control {
            self.selection.toggle(path);
        } else if modifiers.shift {
            let ordered = self.ordered_paths(cx);
            self.selection.extend_to(&path, &ordered);
        } else {
            self.selection.select_only(path);
        }
        window.focus(&self.focus);
        cx.notify();
    }

    fn open_item(&mut self, path: PathBuf, is_dir: bool, cx: &mut Context<Self>) {
        if is_dir {
            self.navigate(path, cx);
        } else if let Err(message) = open_path(&path) {
            self.show_message(message, cx);
        }
    }

    pub fn show_message(&mut self, message: String, cx: &mut Context<Self>) {
        self.message = Some(message);
        cx.notify();
    }

    fn open_selection(&mut self, cx: &mut Context<Self>) {
        for path in self.selection.paths() {
            self.open_item(path.clone(), path.is_dir(), cx);
        }
    }

    /// 項目の共通のマウス操作。押下の記録、右クリック、ドロップの受け入れ。
    pub fn item_mouse(&self, row: Stateful<gpui::Div>, path: PathBuf, is_dir: bool, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let (press_path, menu_path, drop_path) = (path.clone(), path.clone(), path);
        row.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if this.renaming.as_ref().is_some_and(|renaming| renaming.path == press_path) {
                    return;
                }
                this.commit_rename(window, cx);
                this.menu = None;
                this.press = Some(RowPress { start: event.position, path: press_path.clone() });
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.item_menu(menu_path.clone(), cx);
            }),
        )
        .when(is_dir, |row| {
            row.drag_over::<ExternalPaths>(|style, _, _, _| style.bg(rgb(theme::HOVER))).on_drop(cx.listener(
                move |this, paths: &ExternalPaths, window, cx| {
                    cx.stop_propagation();
                    let copy = window.modifiers().control;
                    this.drop_paths(paths.paths(), drop_path.clone(), copy, cx);
                },
            ))
        })
    }

    /// ボタンを押したまま動かしたら、OLEのドラッグを始める。
    pub fn drag_row(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(press) = &self.press else { return };
        let moved: f32 = (event.position - press.start).magnitude() as f32;
        if moved <= theme::DRAG_THRESHOLD {
            return;
        }
        let Some(press) = self.press.take() else { return };
        if !self.selection.contains(&press.path) {
            self.selection.select_only(press.path.clone());
        }
        let Some((folder, names)) = self.selection_in_folder_of(&press.path) else { return };
        let hwnd = self.hwnd;
        cx.notify();
        cx.defer(move |_| {
            if let Err(error) = drag_items(hwnd, &folder, &names) {
                eprintln!("ドラッグを始められませんでした: {error}");
            }
        });
    }

    pub fn end_press(&mut self) {
        self.press = None;
    }

    /// `path` と同じフォルダにある選択項目。シェルのメニューとドラッグは1フォルダ単位で動く。
    fn selection_in_folder_of(&self, path: &Path) -> Option<(PathBuf, Vec<OsString>)> {
        let folder = path.parent()?.to_path_buf();
        let names = self
            .selection
            .paths()
            .iter()
            .filter(|item| item.parent() == Some(folder.as_path()))
            .filter_map(|item| Some(item.file_name()?.to_os_string()))
            .collect();
        Some((folder, names))
    }

    fn item_menu(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.selection.contains(&path) {
            self.selection.select_only(path.clone());
        }
        let Some((folder, names)) = self.selection_in_folder_of(&path) else { return };
        let hwnd = self.hwnd;
        cx.notify();
        cx.defer(move |_| {
            if let Err(error) = show_items_menu(hwnd, &folder, &names) {
                eprintln!("メニューを出せませんでした: {error}");
            }
        });
    }

    /// 余白の右クリック。新規作成や貼り付けのメニュー。
    pub fn folder_menu(&mut self, cx: &mut Context<Self>) {
        let (hwnd, folder) = (self.hwnd, self.current_folder(cx));
        cx.defer(move |_| {
            if let Err(error) = show_folder_menu(hwnd, &folder) {
                eprintln!("メニューを出せませんでした: {error}");
            }
        });
    }

    /// エクスプローラーと同じく、同じドライブなら移動、別なら複写にする。Ctrlは複写。
    pub fn drop_paths(&mut self, paths: &[PathBuf], target: PathBuf, copy: bool, cx: &mut Context<Self>) {
        let items: Vec<PathBuf> =
            paths.iter().filter(|path| path.parent() != Some(target.as_path()) && **path != target).cloned().collect();
        let Some(first) = items.first() else { return };
        let operation = if copy || !same_volume(first, &target) {
            Operation::Copy { items, destination: target }
        } else {
            Operation::Move { items, destination: target }
        };
        self.perform(operation, true, cx);
    }

    /// ファイル操作を裏で実行し、終わったら表示を更新する。
    pub fn perform(&mut self, operation: Operation, record_undo: bool, cx: &mut Context<Self>) {
        let hwnd = self.hwnd;
        let (tx, rx) = futures::channel::oneshot::channel();
        let job = operation.clone();
        std::thread::spawn(move || {
            init_thread();
            let _ = tx.send(run_operation(&job, Prompt::Windows { hwnd }));
        });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("ファイル操作が途中で止まりました".to_string()));
            let _ = this.update(cx, |browser, cx| browser.finish_operation(&operation, result, record_undo, cx));
        })
        .detach();
    }

    fn finish_operation(&mut self, operation: &Operation, result: Result<(), String>, record_undo: bool, cx: &mut Context<Self>) {
        match result {
            Ok(()) => {
                if record_undo && let Some(undo) = inverse(operation) {
                    self.undo.push(undo);
                }
                self.message = None;
            }
            Err(message) => self.message = Some(message),
        }
        let panes = std::iter::once(self.pane.clone())
            .chain(self.columns.iter().map(|column| column.pane.clone()))
            .chain(self.expanded.values().cloned());
        for pane in panes.collect::<Vec<_>>() {
            pane.update(cx, |pane, cx| pane.reload(cx));
        }
        cx.notify();
    }

    fn undo_last(&mut self, cx: &mut Context<Self>) {
        if let Some(operation) = self.undo.pop() {
            self.perform(operation, false, cx);
        }
    }

    fn copy_selection(&mut self, cut: bool, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            return;
        }
        if let Err(message) = set_clipboard_files(&self.selection.paths(), cut) {
            self.show_message(message, cx);
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some((items, cut)) = clipboard_files() else { return };
        let destination = self.current_folder(cx);
        let operation = if cut { Operation::Move { items, destination } } else { Operation::Copy { items, destination } };
        self.perform(operation, true, cx);
    }

    fn recycle_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selection.is_empty() {
            let items = self.selection.paths();
            self.selection.clear();
            self.perform(Operation::Recycle { items }, false, cx);
        }
    }

    fn new_folder(&mut self, cx: &mut Context<Self>) {
        let parent = self.current_folder(cx);
        let name = unique_folder_name(&parent);
        self.pending_rename = Some(parent.join(&name));
        self.perform(Operation::NewFolder { parent, name }, false, cx);
    }

    /// 名前の編集を始める。拡張子を除いた部分を選んだ状態にする。
    pub fn begin_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else { return };
        let stem = match name.rfind('.') {
            Some(dot) if dot > 0 && !path.is_dir() => dot,
            _ => name.len(),
        };
        let input = cx.new(|cx| TextInput::new(name, 0..stem, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::Submit => this.commit_rename(window, cx),
            InputEvent::Cancel => this.end_rename(window, cx),
        });
        window.focus(&input.focus_handle(cx));
        self.renaming = Some(Renaming { path, input, _subscription: subscription });
        cx.notify();
    }

    fn end_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.renaming = None;
        remove_ime_caret();
        window.focus(&self.focus);
        cx.notify();
    }

    /// 編集中の名前を確定する。編集していなければ何もしない。
    pub fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = &self.renaming else { return };
        let name = renaming.input.read(cx).text();
        let item = renaming.path.clone();
        let unchanged = item.file_name().is_some_and(|old| old.to_string_lossy() == name);
        self.end_rename(window, cx);
        if unchanged {
            return;
        }
        if !valid_file_name(&name) {
            self.show_message(format!("「{name}」は名前に使えません"), cx);
            return;
        }
        self.selection.select_only(item.with_file_name(&name));
        self.perform(Operation::Rename { item, name }, true, cx);
    }

    fn reveal(&mut self, row: usize) {
        match self.view.mode {
            ViewMode::List => self.list_scroll.scroll_to_item(row, ScrollStrategy::Top),
            ViewMode::Columns => self.columns[self.active_column].scroll.scroll_to_item(row, ScrollStrategy::Top),
            ViewMode::Icons => {}
        }
    }

    fn move_selection(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        let items = self.visible_items(cx);
        let ordered: Vec<PathBuf> = items.iter().map(|(_, path)| path.clone()).collect();
        let Some(path) = self.selection.move_cursor(delta, extend, &ordered) else { return };
        if let Some((row, _)) = items.iter().find(|(_, item)| *item == path) {
            self.reveal(*row);
        }
        if self.view.mode == ViewMode::Columns && !extend {
            self.follow_in_column(&path, cx);
        }
        cx.notify();
    }

    /// カラムで選択を動かした時、右の列も追従させる。
    fn follow_in_column(&mut self, path: &Path, cx: &mut Context<Self>) {
        let column = self.active_column;
        let state = &self.columns[column].pane.read(cx).state;
        let Some(name) = path.file_name() else { return };
        let Some(entry) = state.entries.iter().find(|entry| entry.os_name() == name).cloned() else { return };
        self.select_in_column(column, name.to_os_string(), entry, cx);
        self.active_column = column;
    }

    fn step_column(&mut self, forward: bool, cx: &mut Context<Self>) {
        if forward {
            if self.active_column + 1 < self.columns.len() {
                self.active_column += 1;
                self.selection.clear();
                self.move_selection(1, false, cx);
            }
        } else if self.active_column > 0 {
            self.active_column -= 1;
            let column = &self.columns[self.active_column];
            let folder = column.pane.read(cx).state.path.clone();
            if let Some(name) = column.selected.clone() {
                self.selection.select_only(folder.join(name));
            }
            self.columns.truncate(self.active_column + 2);
            cx.notify();
        }
    }

    pub fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            return;
        }
        let keystroke = &event.keystroke;
        let (control, shift, alt) = (keystroke.modifiers.control, keystroke.modifiers.shift, keystroke.modifiers.alt);
        let rows = if self.view.mode == ViewMode::Icons { self.grid_per_row as isize } else { 1 };
        let mode = self.view.mode;
        match keystroke.key.as_str() {
            "up" if alt => self.parent(cx),
            "left" if alt => self.back(cx),
            "right" if alt => self.forward(cx),
            "up" => self.move_selection(-rows, shift, cx),
            "down" => self.move_selection(rows, shift, cx),
            "left" if mode == ViewMode::Icons => self.move_selection(-1, shift, cx),
            "right" if mode == ViewMode::Icons => self.move_selection(1, shift, cx),
            "left" if mode == ViewMode::Columns => self.step_column(false, cx),
            "right" if mode == ViewMode::Columns => self.step_column(true, cx),
            "enter" => self.open_selection(cx),
            "backspace" => self.back(cx),
            "escape" => self.clear_selection(cx),
            "delete" => self.recycle_selection(cx),
            "f2" => {
                if let [path] = self.selection.paths().as_slice() {
                    self.begin_rename(path.clone(), window, cx);
                }
            }
            "f5" => self.pane.update(cx, |pane, cx| pane.reload(cx)),
            "a" if control => {
                let ordered = self.ordered_paths(cx);
                self.selection.select_all(&ordered);
                cx.notify();
            }
            "c" if control => self.copy_selection(false, cx),
            "x" if control => self.copy_selection(true, cx),
            "v" if control => self.paste(cx),
            "z" if control => self.undo_last(cx),
            "n" if control && shift => self.new_folder(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

#[cfg(test)]
mod tests {
    use super::same_volume;
    use std::path::Path;

    #[test]
    fn volumes_compare_by_drive_or_share() {
        assert!(same_volume(Path::new(r"C:\a"), Path::new(r"C:\b\c")));
        assert!(!same_volume(Path::new(r"C:\a"), Path::new(r"E:\a")));
        assert!(same_volume(Path::new(r"\\nas\share\a"), Path::new(r"\\nas\share\b")));
    }

    #[test]
    fn items_keep_row_numbers_around_headers() {
        use super::items_of;
        use crate::list_tree::{Flattened, Row, TreeRow};
        use filer_core::Entry;
        use std::collections::HashMap;
        let entry = Entry { name: "a".into(), raw_name: None, is_dir: false, is_link: false, size: 0, modified: 0, created: 0 };
        let tree = Flattened {
            folders: vec!["r".into()],
            rows: vec![
                Row::Header { label: "h".into(), count: 1 },
                Row::Item(TreeRow { depth: 0, folder: 0, entry: 0 }),
            ],
        };
        let entries = [entry];
        let lists = HashMap::from([(Path::new("r"), &entries[..])]);
        assert_eq!(items_of(&tree, &lists), [(1, Path::new("r").join("a"))]);
    }
}

