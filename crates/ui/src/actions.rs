//! 選択、キー操作、ファイル操作など、画面の表示以外の動作。

use crate::browser::Browser;
use crate::icons_view::{GridRow, grid_item_rects, grid_metrics};
use crate::list_tree::{Flattened, Row, group_range};
use crate::parts::{Icon, icon};
use crate::text_input::{InputEvent, TextInput};
use crate::theme;
use filer_core::{Chord, Command, Entry, Gesture, ViewMode, is_favorite, rows_in_band};
use filer_shell::{
    MenuChoice, Opened, Operation, Prompt, clipboard_files, cursor_position, drag_items, init_thread, left_button_down, open_target, remove_ime_caret, run_recording_undo,
    set_clipboard_files, show_folder_menu, show_items_menu, unique_folder_name, valid_file_name,
};
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, CursorStyle, DragMoveEvent, Entity, ExternalPaths, Focusable, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, Point, ScrollStrategy, Stateful, StyleRefinement, Subscription, Task, Window, div, point, prelude::*, px, rgb, rgba,
};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

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
    /// アイコンと名前の外を押した時は、ファイルを掴まず範囲選択を始める
    band_column: Option<Option<usize>>,
}

/// ドラッグ中に止まると開く先。
#[derive(Clone, PartialEq, Eq)]
pub enum SpringTarget {
    /// 一覧の中のフォルダ
    Folder(PathBuf),
    /// サイドバーの項目やパスの帯
    Go(PathBuf),
    Back,
    Parent,
    /// タブの帯の1枚
    Tab(u64),
}

/// ドラッグ中に止まっている先。時間が来たら開く。
pub struct Spring {
    target: SpringTarget,
    /// 同じ行き先の部品が他にもあるので、どの部品の上かで見分ける
    bounds: Bounds<Pixels>,
    /// 開く前の点滅で、青を見せている
    blue: bool,
    _task: Task<()>,
}

/// 右クリックの「新規作成」で増える項目を待つ間の状態。
pub struct NewItems {
    folder: PathBuf,
    known: HashSet<OsString>,
    until: Instant,
}

/// ショートカットのファイルか。
pub fn is_shortcut(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
}

/// 範囲選択の最中の状態。始点はスクロールしても動かないよう、中身の座標で持つ。
pub struct Band {
    start: Point<Pixels>,
    current: Point<Pixels>,
    column: Option<usize>,
    base: Vec<PathBuf>,
}

struct BandViewport {
    bounds: Bounds<Pixels>,
    scroll: Pixels,
}

const BAND_TICK: Duration = Duration::from_millis(16);
/// 「新規作成」の結果が一覧に現れるのを待つ時間
const NEW_ITEM_WAIT: Duration = Duration::from_secs(5);
/// 右クリックのメニューの先頭に足す項目。
const ADD_FAVORITE: &str = "よく使う項目に追加";

/// 2つのパスが同じドライブ（または共有）にあるか。
fn same_volume(a: &Path, b: &Path) -> bool {
    a.components().next() == b.components().next()
}

/// ドラッグしている項目か、その中にあるフォルダか。自分の中へは入れない。
fn carries(dragged: &[PathBuf], folder: &Path) -> bool {
    dragged.iter().any(|path| folder.starts_with(path))
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
        self.tab.folder(cx)
    }

    /// 画面に並んでいる順の項目。
    fn visible_items(&mut self, cx: &Context<Self>) -> Vec<(usize, PathBuf)> {
        match self.tab.view.mode {
            ViewMode::List => {
                let tree = self.tree(cx);
                let root = self.tab.pane.read(cx);
                let mut lists = HashMap::from([(root.state.path.as_path(), root.state.entries.as_slice())]);
                for (path, pane) in &self.tab.expanded {
                    lists.insert(path.as_path(), pane.read(cx).state.entries.as_slice());
                }
                items_of(&tree, &lists)
            }
            ViewMode::Columns => {
                let tree = self.column_tree(self.tab.active_column, cx);
                let state = &self.tab.columns[self.tab.active_column].pane.read(cx).state;
                items_of(&tree, &HashMap::from([(state.path.as_path(), state.entries.as_slice())]))
            }
            ViewMode::Icons => {
                let pane = self.tab.pane.read(cx);
                let (_, tree) = Self::plain_tree(pane, self.tab.view.group);
                let state = &pane.state;
                items_of(&tree, &HashMap::from([(state.path.as_path(), state.entries.as_slice())]))
            }
        }
    }

    /// 今読み込まれている項目のすべて。隠れている親の列も含む。
    fn known_paths(&self, cx: &Context<Self>) -> Vec<PathBuf> {
        let columns = self.tab.columns.iter().map(|column| &column.pane);
        let panes = std::iter::once(&self.tab.pane).chain(self.tab.expanded.values()).chain(columns);
        let lists = panes.map(|pane| &pane.read(cx).state);
        lists.flat_map(|state| state.entries.iter().map(|entry| state.path.join(entry.os_name()))).collect()
    }

    /// 選択の中の、よく使う項目に無いフォルダ。
    fn unregistered_folders(&self, cx: &Context<Self>) -> Vec<PathBuf> {
        let selected: HashSet<PathBuf> = self.tab.selection.paths().iter().cloned().collect();
        let columns = self.tab.columns.iter().map(|column| &column.pane);
        let panes = std::iter::once(&self.tab.pane).chain(self.tab.expanded.values()).chain(columns);
        let mut folders = Vec::new();
        for pane in panes {
            let state = &pane.read(cx).state;
            for entry in state.entries.iter().filter(|entry| entry.is_dir) {
                let path = state.path.join(entry.os_name());
                if selected.contains(&path) && !folders.contains(&path) && !is_favorite(&self.favorites, &path) {
                    folders.push(path);
                }
            }
        }
        folders
    }

    /// 消えた項目や、閉じたフォルダの中の項目を選択から外す。
    /// 新規フォルダの一覧への反映を待って、名前の編集を始める。
    pub fn sync_with_listing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.tab.pane.read(cx);
        let version = self.tab.expanded.values().map(|pane| pane.read(cx).version).sum::<u64>()
            + self.tab.columns.iter().map(|column| column.pane.read(cx).version).sum::<u64>()
            + root.version
            + self.tab.expanded.len() as u64;
        if version == self.tab.synced_version || root.state.loading {
            return;
        }
        self.tab.synced_version = version;
        self.rename_new_item(window, cx);
        let known = self.known_paths(cx);
        self.tab.selection.retain_existing(&known);
        if let Some(path) = self.tab.pending_rename.take_if(|path| known.contains(path)) {
            self.tab.selection.select_only(path.clone());
            self.begin_rename(path, window, cx);
        }
        // 隠し項目などで一覧に無ければ、選ばずに諦める
        if let Some(path) = self.tab.pending_select.take().filter(|path| known.contains(path)) {
            self.tab.selection.select_only(path.clone());
            if let Some((row, _)) = self.visible_items(cx).iter().find(|(_, item)| *item == path) {
                self.reveal(*row);
            }
        }
    }

    /// 「新規作成」で増えた項目を見つけたら、名前の編集を始める。
    fn rename_new_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab.new_items.as_ref().is_some_and(|wait| wait.until < Instant::now()) {
            self.tab.new_items = None;
        }
        let Some(wait) = &self.tab.new_items else { return };
        let Some(column) = self.tab.columns.iter().find(|column| column.pane.read(cx).state.path == wait.folder) else { return };
        let added = column.pane.read(cx).state.entries.iter().find(|entry| !wait.known.contains(entry.os_name()));
        let Some(path) = added.map(|entry| wait.folder.join(entry.os_name())) else { return };
        self.tab.new_items = None;
        self.tab.selection.select_only(path.clone());
        self.begin_rename(path, window, cx);
    }

    pub fn ordered_paths(&mut self, cx: &Context<Self>) -> Vec<PathBuf> {
        self.visible_items(cx).into_iter().map(|(_, path)| path).collect()
    }

    /// クリックでの選択。Ctrlは追加、Shiftは範囲。ダブルクリックは開く。
    pub fn click_item(&mut self, path: PathBuf, is_dir: bool, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.suppress_click) {
            return;
        }
        if self.tab.renaming.as_ref().is_some_and(|renaming| renaming.path == path) {
            return;
        }
        let modifiers = event.modifiers();
        if event.click_count() == 2 && !modifiers.control && !modifiers.shift {
            self.open_item(path, is_dir, cx);
            return;
        }
        if event.click_count() == 2 && modifiers.control && !modifiers.shift && (is_dir || is_shortcut(&path)) {
            self.item_gesture(Gesture::ControlDoubleClick, path, is_dir, cx);
            return;
        }
        if modifiers.control {
            self.tab.selection.toggle(path);
        } else if modifiers.shift {
            let ordered = self.ordered_paths(cx);
            self.tab.selection.extend_to(&path, &ordered);
        } else {
            self.tab.selection.select_only(path);
        }
        window.focus(&self.focus);
        cx.notify();
    }

    fn open_item(&mut self, path: PathBuf, is_dir: bool, cx: &mut Context<Self>) {
        if is_dir {
            if self.tab.view.mode != ViewMode::Columns || !self.pin_folder(&path, cx) {
                self.navigate(path, cx);
            }
            return;
        }
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            init_thread();
            let _ = tx.send(open_target(&path));
        });
        let tab = self.tab.id;
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("開く処理が途中で止まりました".to_string()));
            let _ = this.update(cx, |browser, cx| match result {
                Ok(Opened::Launched) => {}
                Ok(Opened::Folder(folder)) => {
                    browser.on_tab(tab, cx, |browser, cx| browser.navigate(folder, cx));
                }
                Err(message) => browser.show_message_on(tab, message, cx),
            });
        })
        .detach();
    }

    pub fn show_message(&mut self, message: String, cx: &mut Context<Self>) {
        self.tab.message = Some(message);
        cx.notify();
    }

    /// 頼んだタブへ伝える。閉じられていたら今のタブへ出す。
    fn show_message_on(&mut self, tab: u64, message: String, cx: &mut Context<Self>) {
        let mut text = Some(message);
        self.on_tab(tab, cx, |browser, _| browser.tab.message = text.take());
        if let Some(message) = text {
            self.tab.message = Some(message);
        }
        cx.notify();
    }

    fn open_selection(&mut self, cx: &mut Context<Self>) {
        for path in self.tab.selection.paths() {
            self.open_item(path.clone(), path.is_dir(), cx);
        }
    }

    /// リストでファイルを掴める範囲。行の高さいっぱいで、短い名前でも一定の幅を取る。
    pub fn grab_zone(&self, zone: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        zone.h_full()
            .min_w(px(theme::GRAB_MIN_WIDTH))
            .cursor(CursorStyle::Arrow)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, _| this.grab_press = true))
    }

    /// ドラッグが乗った時の背景。文字が白い選択行に灰色を敷くと読めないので、濃い青にする。
    fn drop_color(&self, path: &Path, cx: &Context<Self>) -> u32 {
        let selected = self.tab.selection.contains(path) || self.is_open_in_column(path, cx);
        if selected && self.selection_colors().1 == theme::SELECTED_TEXT { theme::SELECTED_DROP } else { theme::HOVER }
    }

    /// 項目の共通のマウス操作。押下の記録、右クリック、ドロップの受け入れ。
    ///
    /// `band` が `Some` の時は、`grab_zone` の外を押すと範囲選択になる。
    pub fn item_mouse(&self, row: Stateful<gpui::Div>, path: PathBuf, is_dir: bool, band: Option<Option<usize>>, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let blue = self.spring_is_blue(&SpringTarget::Folder(path.clone()));
        let drop_color = if blue { theme::SELECTED } else { self.drop_color(&path, cx) };
        let (press_path, menu_path, tab_path, drop_path) = (path.clone(), path.clone(), path.clone(), path);
        row.on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.item_gesture(Gesture::MiddleClick, tab_path.clone(), is_dir, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let grabbed = std::mem::take(&mut this.grab_press);
                if this.tab.renaming.as_ref().is_some_and(|renaming| renaming.path == press_path) {
                    return;
                }
                this.commit_rename(window, cx);
                this.menu = None;
                this.suppress_click = false;
                let band_column = band.filter(|_| !grabbed);
                this.press = Some(RowPress { start: event.position, path: press_path.clone(), band_column });
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
            let (spring_path, over_path) = (drop_path.clone(), drop_path.clone());
            row.drag_over::<ExternalPaths>(move |style, paths: &ExternalPaths, _, _| {
                if carries(paths.paths(), &over_path) { style } else { Self::drag_style(style, drop_color) }
            })
            .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                if !carries(event.drag(cx).paths(), &spring_path) {
                    let target = SpringTarget::Folder(spring_path.clone());
                    this.hover_target(target, event.bounds, event.event.position, cx);
                }
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                cx.stop_propagation();
                let copy = filer_shell::control_down();
                this.drop_paths(paths.paths(), drop_path.clone(), copy, cx);
            }))
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
        if let Some(column) = press.band_column {
            // 離した時の行のクリックで、選んだ範囲が消えないようにする
            self.suppress_click = true;
            match column {
                Some(column) => self.clear_column(column, cx),
                None => self.tab.selection.clear(),
            }
            self.begin_band(press.start, column, event.modifiers.control, cx);
            return;
        }
        if !self.tab.selection.contains(&press.path) {
            self.select_pressed(press.path.clone(), cx);
        }
        let Some((folder, names)) = self.selection_in_folder_of(&press.path) else { return };
        let hwnd = self.hwnd;
        let label = match names.as_slice() {
            [name] => name.to_string_lossy().into_owned(),
            names => format!("{} 項目", names.len()),
        };
        cx.notify();
        Self::outside_update(cx, move || {
            if let Err(error) = drag_items(hwnd, &folder, &names, &label) {
                eprintln!("ドラッグを始められませんでした: {error}");
            }
        });
    }

    pub fn end_press(&mut self) {
        self.press = None;
    }

    /// メニューやドラッグは独自のメッセージループを回す。その間に裏の処理が画面を更新できるよう、
    /// gpuiの更新処理の外で動かす。`cx.defer` の中だと更新が重なって落ちる。
    fn outside_update(cx: &mut Context<Self>, modal: impl FnOnce() + 'static) {
        cx.spawn(async move |_, _| modal()).detach();
    }

    /// 右クリックメニューを出す。「名前の変更」が選ばれたらアプリの入力欄を開く。
    fn run_menu(cx: &mut Context<Self>, rename: Option<PathBuf>, created: Option<NewItems>, favorites: Vec<PathBuf>, modal: impl FnOnce() -> Result<MenuChoice, windows::core::Error> + 'static) {
        cx.spawn(async move |this, cx| {
            let choice = modal();
            let _ = this.update(cx, |browser, cx| {
                match choice {
                    Ok(MenuChoice::Rename) => browser.tab.rename_request = rename,
                    Ok(MenuChoice::Created) => {
                        browser.tab.new_items = created.map(|wait| NewItems { until: Instant::now() + NEW_ITEM_WAIT, ..wait });
                        // 一覧が先に更新されていても、次の描画で探す
                        browser.tab.synced_version = 0;
                    }
                    Ok(MenuChoice::Extra) => browser.add_favorites(favorites, cx),
                    Ok(_) => {}
                    Err(error) => eprintln!("メニューを出せませんでした: {error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `path` と同じフォルダにある選択項目。シェルのメニューとドラッグは1フォルダ単位で動く。
    fn selection_in_folder_of(&self, path: &Path) -> Option<(PathBuf, Vec<OsString>)> {
        let folder = path.parent()?.to_path_buf();
        let names = self
            .tab
            .selection
            .paths()
            .iter()
            .filter(|item| item.parent() == Some(folder.as_path()))
            .filter_map(|item| Some(item.file_name()?.to_os_string()))
            .collect();
        Some((folder, names))
    }

    fn item_menu(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.tab.selection.contains(&path) {
            self.select_pressed(path.clone(), cx);
        }
        let Some((folder, names)) = self.selection_in_folder_of(&path) else { return };
        let hwnd = self.hwnd;
        let favorites = self.unregistered_folders(cx);
        let extra = (!favorites.is_empty()).then_some(ADD_FAVORITE);
        cx.notify();
        Self::run_menu(cx, Some(path), None, favorites, move || show_items_menu(hwnd, &folder, &names, extra));
    }

    /// 余白の右クリック。新規作成や貼り付けのメニュー。
    pub fn folder_menu(&mut self, cx: &mut Context<Self>) {
        let (hwnd, folder) = (self.hwnd, self.current_folder(cx));
        let known = self
            .tab
            .columns
            .iter()
            .find(|column| column.pane.read(cx).state.path == folder)
            .map(|column| column.pane.read(cx).state.entries.iter().map(|entry| entry.os_name().to_os_string()).collect())
            .unwrap_or_default();
        let created = NewItems { folder: folder.clone(), known, until: Instant::now() };
        let favorites = if is_favorite(&self.favorites, &folder) { Vec::new() } else { vec![folder.clone()] };
        let extra = (!favorites.is_empty()).then_some(ADD_FAVORITE);
        Self::run_menu(cx, None, Some(created), favorites, move || show_folder_menu(hwnd, &folder, extra));
    }

    /// エクスプローラーと同じく、同じドライブなら移動、別なら複写にする。Ctrlは複写。
    pub fn drop_paths(&mut self, paths: &[PathBuf], target: PathBuf, copy: bool, cx: &mut Context<Self>) {
        self.end_spring();
        let items: Vec<PathBuf> =
            paths.iter().filter(|path| path.parent() != Some(target.as_path()) && !target.starts_with(path)).cloned().collect();
        let Some(first) = items.first() else { return };
        let operation = if copy || !same_volume(first, &target) {
            Operation::Copy { items, destination: target }
        } else {
            Operation::Move { items, destination: target }
        };
        self.perform(operation, true, cx);
    }

    /// ドラッグが乗った部品の色。開く前の点滅の間は青と交互になる。
    pub fn drag_fill(&self, target: &SpringTarget) -> u32 {
        if self.spring_is_blue(target) { theme::SELECTED } else { theme::HOVER }
    }

    fn spring_is_blue(&self, target: &SpringTarget) -> bool {
        self.spring.as_ref().is_some_and(|spring| spring.blue && spring.target == *target)
    }

    /// ドラッグが乗った時の見た目。青の上は文字を白くする。
    pub fn drag_style(style: StyleRefinement, fill: u32) -> StyleRefinement {
        let style = style.bg(rgb(fill));
        if fill == theme::SELECTED { style.text_color(rgb(theme::SELECTED_TEXT)) } else { style }
    }

    /// 見えている行のフォルダが空か。分かるまでは偽。
    pub fn is_empty_folder(&self, path: &Path, entry: &Entry, cx: &mut Context<Self>) -> bool {
        entry.is_dir && self.empties.update(cx, |empties, _| empties.get(path, entry.modified))
    }

    /// 空のフォルダの行に添える印。
    pub fn empty_mark() -> impl IntoElement {
        div().flex_none().text_size(px(theme::SMALL_TEXT_SIZE)).opacity(0.6).child("空")
    }

    /// ファイル操作を裏で実行し、終わったら表示を更新する。
    pub fn perform(&mut self, operation: Operation, record_undo: bool, cx: &mut Context<Self>) {
        let hwnd = self.hwnd;
        let (tx, rx) = futures::channel::oneshot::channel();
        let job = operation.clone();
        let tab = self.tab.id;
        std::thread::spawn(move || {
            init_thread();
            let _ = tx.send(run_recording_undo(&job, Prompt::Windows { hwnd }));
        });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("ファイル操作が途中で止まりました".to_string()));
            let _ = this.update(cx, |browser, cx| browser.finish_operation(tab, &operation, result, record_undo, cx));
        })
        .detach();
    }

    fn finish_operation(&mut self, tab: u64, operation: &Operation, result: Result<Option<Operation>, String>, record_undo: bool, cx: &mut Context<Self>) {
        match result {
            Ok(undo) => {
                match undo {
                    Some(undo) if record_undo => self.undo.push(undo),
                    // 戻せない操作の後で、さらに前の操作を戻すと順番が崩れる
                    None if record_undo => self.undo.clear(),
                    _ => {}
                }
                if matches!(operation, Operation::Move { .. }) {
                    self.cut.clear();
                }
                self.on_tab(tab, cx, |browser, _| {
                    if let Operation::NewFolder { parent, name } = operation {
                        browser.tab.pending_rename = Some(parent.join(name));
                    }
                    browser.tab.message = None;
                });
            }
            Err(message) => self.show_message_on(tab, message, cx),
        }
        for pane in self.all_panes() {
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
        if self.tab.selection.is_empty() {
            return;
        }
        let items = self.tab.selection.top_level();
        match set_clipboard_files(&items, cut) {
            Ok(()) => self.cut = if cut { items.into_iter().collect() } else { HashSet::new() },
            Err(message) => self.tab.message = Some(message),
        }
        cx.notify();
    }

    /// 切り取った項目か。名前とアイコンを薄くして示す。
    pub fn is_cut(&self, path: &Path) -> bool {
        self.cut.contains(path)
    }

    /// グループの見出しのクリック。そのグループの項目をすべて選ぶ。
    pub fn select_group(&mut self, ordinal: usize, column: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(column) = column {
            self.tab.active_column = column;
        }
        let tree = self.active_tree(cx);
        let range = group_range(&tree, ordinal);
        let rows = self.visible_items(cx).into_iter();
        let members: Vec<PathBuf> = rows.filter(|(ix, _)| range.contains(ix)).map(|(_, path)| path).collect();
        self.tab.selection.select_all(&members);
        window.focus(&self.focus);
        cx.notify();
    }

    fn active_tree(&mut self, cx: &Context<Self>) -> Rc<Flattened> {
        match self.tab.view.mode {
            ViewMode::List => self.tree(cx),
            ViewMode::Columns => self.column_tree(self.tab.active_column, cx),
            ViewMode::Icons => Rc::new(Self::plain_tree(self.tab.pane.read(cx), self.tab.view.group).1),
        }
    }

    /// カーソルがドラッグ中の帯の上にあるか。帯の下の行は、隠れていても位置の上では重なる。
    fn over_drag_bar(&self, position: Point<Pixels>) -> bool {
        let top = self.viewport.height - px(theme::STATUS_HEIGHT + theme::DRAG_BAR_HEIGHT);
        position.y >= top && position.x >= px(self.sidebar_width())
    }

    /// ドラッグ中に、開く先の上で止まったら少し待って開く。
    pub fn hover_target(&mut self, target: SpringTarget, bounds: Bounds<Pixels>, position: Point<Pixels>, cx: &mut Context<Self>) {
        let covered = matches!(target, SpringTarget::Folder(_)) && self.over_drag_bar(position);
        let here = self.spring_is_done(&target, cx);
        let inside = bounds.contains(&position) && !covered && !here;
        let current = self.spring.as_ref().is_some_and(|spring| spring.target == target && spring.bounds == bounds);
        if inside == current {
            return;
        }
        if !inside {
            self.spring = None;
            cx.notify();
            return;
        }
        let delay = self.spring_delay(&target);
        let (interval, count, hold) = (self.settings.flash_interval_ms, self.settings.flash_count, self.settings.flash_hold_ms);
        let fired = target.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(delay)).await;
            if interval > 0 && count > 0 {
                // 青から始めて青で終わり、最後の青だけを長く見せる
                let last = count * 2 - 2;
                for step in 0..=last {
                    let _ = this.update(cx, |browser, cx| {
                        if let Some(spring) = &mut browser.spring {
                            spring.blue = step % 2 == 0;
                        }
                        cx.notify();
                    });
                    let wait = if step == last { hold } else { interval };
                    cx.background_executor().timer(Duration::from_millis(wait)).await;
                }
            }
            let _ = this.update(cx, |browser, cx| browser.open_hovered(fired, cx));
        });
        self.spring = Some(Spring { target, bounds, blue: false, _task: task });
    }

    /// 止まっても何も変わらない行き先か。開いた後に点滅を繰り返さないようにする。
    fn spring_is_done(&self, target: &SpringTarget, cx: &Context<Self>) -> bool {
        match target {
            SpringTarget::Tab(id) => self.tab_is_current(*id),
            SpringTarget::Go(path) => self.current_folder(cx) == *path,
            SpringTarget::Folder(path) => match self.tab.view.mode {
                ViewMode::List => self.tab.expanded.contains_key(path),
                ViewMode::Columns => self.is_open_in_column(path, cx),
                ViewMode::Icons => false,
            },
            _ => false,
        }
    }

    /// 開くまでの待ち時間。開き方ごとに設定できる。
    fn spring_delay(&self, target: &SpringTarget) -> u64 {
        match target {
            SpringTarget::Folder(_) => match self.tab.view.mode {
                ViewMode::Columns => self.settings.spring_column_ms,
                ViewMode::List => self.settings.spring_list_ms,
                ViewMode::Icons => self.settings.spring_icons_ms,
            },
            _ if self.springs_done == 0 => self.settings.spring_first_ms,
            _ => self.settings.spring_next_ms,
        }
    }

    /// 止まった時の移動先のフォルダ。行の展開やタブの切り替えでは `None`。
    fn spring_destination(&self, target: &SpringTarget, cx: &Context<Self>) -> Option<PathBuf> {
        match target {
            SpringTarget::Back if !self.column_pinned() => self.tab.history.peek_back().map(PathBuf::from),
            SpringTarget::Back | SpringTarget::Parent => self.current_folder(cx).parent().map(PathBuf::from),
            SpringTarget::Go(path) => Some(path.clone()),
            SpringTarget::Folder(_) | SpringTarget::Tab(_) => None,
        }
    }

    fn open_hovered(&mut self, target: SpringTarget, cx: &mut Context<Self>) {
        self.spring = None;
        if !cx.has_active_drag() {
            return;
        }
        self.springs_done += 1;
        // 行き先を開いているタブがあれば、今のタブのパスを変えずにそちらへ移る
        let open = self.spring_destination(&target, cx).and_then(|folder| self.tab_showing(&folder, cx));
        if let Some(id) = open {
            self.switch_to_hovered(id, cx);
            return;
        }
        match target {
            SpringTarget::Back => self.back(cx),
            SpringTarget::Parent => self.parent(cx),
            SpringTarget::Go(path) => self.navigate(path, cx),
            SpringTarget::Tab(id) => self.switch_to_hovered(id, cx),
            SpringTarget::Folder(path) => match self.tab.view.mode {
                ViewMode::Columns => self.open_column_for(&path, cx),
                ViewMode::List => {
                    if !self.tab.expanded.contains_key(&path) {
                        let pane = self.new_pane(path.clone(), self.tab.view.order, false, cx);
                        self.tab.expanded.insert(path, pane);
                    }
                }
                ViewMode::Icons => self.navigate(path, cx),
            },
        }
        cx.notify();
    }

    pub fn open_column_for(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(parent) = path.parent() else { return };
        let Some(name) = path.file_name() else { return };
        let found = self.tab.columns.iter().position(|column| column.pane.read(cx).state.path == parent);
        let Some(column) = found else { return };
        let state = &self.tab.columns[column].pane.read(cx).state;
        let Some(entry) = state.entries.iter().find(|entry| entry.os_name() == name).cloned() else { return };
        self.select_in_column(column, name.to_os_string(), entry, cx);
    }

    /// ドラッグの終わりに、待っている開閉を片付ける。
    pub fn end_spring(&mut self) {
        self.spring = None;
        self.springs_done = 0;
    }

    /// 余白を押した時に、範囲選択を始める。カーソルはウィンドウの外でも追う。
    pub fn begin_band(&mut self, position: Point<Pixels>, column: Option<usize>, control: bool, cx: &mut Context<Self>) {
        let Some(view) = self.band_viewport(column) else { return };
        let start = point(position.x - view.bounds.origin.x, position.y - view.bounds.origin.y - view.scroll);
        let base = if control { self.tab.selection.paths() } else { Vec::new() };
        self.band = Some(Band { start, current: position, column, base });
        self.band_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BAND_TICK).await;
                let running = this.update(cx, |browser, cx| browser.tick_band(cx));
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
    }

    fn band_viewport(&self, column: Option<usize>) -> Option<BandViewport> {
        let (bounds, scroll) = match self.tab.view.mode {
            ViewMode::List => {
                let handle = self.tab.list_scroll.0.borrow().base_handle.clone();
                (handle.bounds(), handle.offset().y)
            }
            ViewMode::Columns => {
                let handle = self.tab.columns.get(column?)?.scroll.0.borrow().base_handle.clone();
                (handle.bounds(), handle.offset().y)
            }
            ViewMode::Icons => (self.tab.grid_list.viewport_bounds(), self.tab.grid_list.scroll_px_offset_for_scrollbar().y),
        };
        Some(BandViewport { bounds, scroll })
    }

    /// 範囲選択の1回分の更新。マウスの位置はOSから読むので、ツールバーや外へ出ても途切れない。
    fn tick_band(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((x, y)) = cursor_position(self.hwnd).filter(|_| left_button_down()) else {
            self.band = None;
            self.band_task = None;
            cx.notify();
            return false;
        };
        let Some(band) = &mut self.band else { return false };
        let current = point(px(x as f32 / self.scale), px(y as f32 / self.scale));
        band.current = current;
        let (column, start, base) = (band.column, band.start, band.base.clone());
        let Some(view) = self.band_viewport(column) else { return true };
        self.scroll_with_band(column, current, view.bounds);
        let Some(view) = self.band_viewport(column) else { return true };
        let end = point(current.x - view.bounds.origin.x, current.y - view.bounds.origin.y - view.scroll);
        // 押しただけでは何も選ばない
        let moved: f32 = (end - start).magnitude() as f32;
        if moved < theme::DRAG_THRESHOLD {
            return true;
        }
        let hits = match self.tab.view.mode {
            ViewMode::Icons => self.grid_hits(start, end, cx),
            ViewMode::List | ViewMode::Columns => {
                if let Some(column) = column {
                    self.tab.active_column = column;
                }
                let tree = self.active_tree(cx);
                let rows = rows_in_band(start.y.into(), end.y.into(), theme::ROW_HEIGHT, tree.rows.len());
                let items = self.visible_items(cx).into_iter();
                items.filter(|(ix, _)| rows.contains(ix)).map(|(_, path)| path).collect()
            }
        };
        self.tab.selection.select_all(&[base, hits].concat());
        self.tab.selection.soften();
        cx.notify();
        true
    }

    /// カーソルが一覧の上下へ出ている間は、離れた距離に応じてスクロールする。
    fn scroll_with_band(&self, column: Option<usize>, cursor: Point<Pixels>, bounds: Bounds<Pixels>) {
        let (y, top, bottom): (f32, f32, f32) = (cursor.y.into(), bounds.top().into(), bounds.bottom().into());
        let beyond = if y > bottom { y - bottom } else if y < top { y - top } else { return };
        let step = (beyond.abs().min(theme::BAND_SCROLL_LIMIT) * theme::BAND_SCROLL_SPEED).max(1.0) * beyond.signum();
        let handle = match self.tab.view.mode {
            ViewMode::Icons => {
                self.tab.grid_list.scroll_by(px(step));
                return;
            }
            ViewMode::List => self.tab.list_scroll.0.borrow().base_handle.clone(),
            ViewMode::Columns => match column.and_then(|column| self.tab.columns.get(column)) {
                Some(column) => column.scroll.0.borrow().base_handle.clone(),
                None => return,
            },
        };
        let offset = handle.offset();
        let lowest = -handle.max_offset().height;
        handle.set_offset(point(offset.x, (offset.y - px(step)).clamp(lowest, px(0.))));
    }

    /// アイコン表示で枠に触れている項目。
    fn grid_hits(&self, start: Point<Pixels>, end: Point<Pixels>, cx: &Context<Self>) -> Vec<PathBuf> {
        let Some(data) = &self.tab.grid_data else { return Vec::new() };
        let (cell_width, row_height) = grid_metrics(self.tab.view.icon_size);
        let (left, right): (f32, f32) = (start.x.min(end.x).into(), start.x.max(end.x).into());
        let (top, bottom): (f32, f32) = (start.y.min(end.y).into(), start.y.max(end.y).into());
        let state = &self.tab.pane.read(cx).state;
        let mut hits = Vec::new();
        let mut y = 0.0;
        for row in data.rows.iter() {
            match row {
                GridRow::Header { .. } => y += theme::ROW_HEIGHT,
                GridRow::Cells(cells) => {
                    if y < bottom && y + row_height > top {
                        for (index, &entry) in cells.iter().enumerate() {
                            let x = theme::GRID_SIDE_PADDING + index as f32 * (cell_width + theme::GRID_CELL_GAP);
                            if let Some(entry) = state.entries.get(entry as usize)
                                && grid_item_rects(entry, self.tab.view.icon_size, x, y).iter().any(|rect| rect.touches(left, top, right, bottom))
                            {
                                hits.push(state.path.join(entry.os_name()));
                            }
                        }
                    }
                    y += row_height;
                }
            }
        }
        hits
    }

    /// 範囲選択の枠。一覧の範囲で切り取り、サイドバーやヘッダーの上には出さない。
    pub fn band_view(&self) -> Option<impl IntoElement + use<>> {
        let band = self.band.as_ref()?;
        let view = self.band_viewport(band.column)?;
        let origin = view.bounds.origin;
        let start = point(band.start.x, band.start.y + view.scroll);
        let current = point(band.current.x - origin.x, band.current.y - origin.y);
        let (x0, x1) = (start.x.min(current.x), start.x.max(current.x));
        let (y0, y1) = (start.y.min(current.y), start.y.max(current.y));
        let area = div()
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .w(view.bounds.size.width)
            .h(view.bounds.size.height)
            .overflow_hidden();
        Some(area.child(
            div()
                .absolute()
                .left(x0)
                .top(y0)
                .w(x1 - x0)
                .h(y1 - y0)
                .bg(rgba(theme::BAND_FILL))
                .border_1()
                .border_color(rgb(theme::BAND_LINE)),
        ))
    }

    /// ドラッグ中に下へ出す大きな帯。戻る先と上の階層へ、広い的で移動できる。
    pub fn drag_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.current_folder(cx);
        let settings = &self.settings;
        let parent = current.parent().map(PathBuf::from).filter(|_| settings.drag_bar_parent);
        // 「上の階層」を出す時は、同じ行き先のフォルダを並べない
        let skipped = 1 + usize::from(parent.is_some());
        let mut ancestors: Vec<PathBuf> = current.ancestors().skip(skipped).take(settings.drag_bar_folders as usize).map(PathBuf::from).collect();
        ancestors.reverse();
        let back = (settings.drag_bar_back && self.tab.history.can_go_back())
            .then(|| self.drag_segment("drag-back".into(), Some(Icon::Back), "戻る".into(), SpringTarget::Back, None, cx));
        let up = parent.map(|parent| self.drag_segment("drag-up".into(), Some(Icon::Up), "上の階層".into(), SpringTarget::Parent, Some(parent), cx));
        let segments: Vec<_> = ancestors
            .into_iter()
            .map(|path| {
                let label = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                let id = format!("drag-go-{}", path.display());
                self.drag_segment(id, None, label, SpringTarget::Go(path.clone()), Some(path), cx)
            })
            .collect();
        div()
            .absolute()
            .left(px(self.sidebar_width()))
            .right_0()
            .bottom(px(theme::STATUS_HEIGHT))
            .h(px(theme::DRAG_BAR_HEIGHT))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .bg(rgb(theme::BAR))
            .border_t_1()
            .border_color(rgb(theme::LINE))
            .shadow_md()
            .occlude()
            .children(back)
            .children(up)
            .children(segments)
    }

    fn drag_segment(&self, id: String, kind: Option<Icon>, label: String, target: SpringTarget, drop: Option<PathBuf>, cx: &mut Context<Self>) -> AnyElement {
        let fill = self.drag_fill(&target);
        div()
            .id(gpui::ElementId::Name(id.into()))
            .flex()
            .flex_1()
            .min_w_0()
            .max_w(px(theme::DRAG_SEGMENT_MAX_WIDTH))
            .items_center()
            .justify_center()
            .gap_1()
            .h(px(theme::DRAG_BAR_HEIGHT - 12.))
            .px_3()
            .rounded(px(theme::CONTROL_RADIUS))
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::LINE))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .drag_over::<ExternalPaths>(move |style, _, _, _| Self::drag_style(style, fill))
            .children(kind.map(|kind| icon(kind, theme::CONTROL_ICON, theme::TEXT)))
            .child(label)
            .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                this.hover_target(target.clone(), event.bounds, event.event.position, cx);
            }))
            .when_some(drop, |segment, path| {
                segment.on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                    cx.stop_propagation();
                    let copy = filer_shell::control_down();
                    this.drop_paths(paths.paths(), path.clone(), copy, cx);
                }))
            })
            .into_any_element()
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some((items, cut)) = clipboard_files() else { return };
        let destination = self.current_folder(cx);
        let operation = if cut { Operation::Move { items, destination } } else { Operation::Copy { items, destination } };
        self.perform(operation, true, cx);
    }

    fn recycle_selection(&mut self, cx: &mut Context<Self>) {
        if !self.tab.selection.is_empty() {
            let items = self.tab.selection.top_level();
            self.tab.selection.clear();
            self.perform(Operation::Recycle { items }, true, cx);
        }
    }

    fn new_folder(&mut self, cx: &mut Context<Self>) {
        let parent = self.current_folder(cx);
        let name = unique_folder_name(&parent);
        self.perform(Operation::NewFolder { parent, name }, true, cx);
    }

    /// 名前の編集を始める。拡張子を除いた部分を選んだ状態にする。
    pub fn begin_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else { return };
        let stem = match name.rfind('.') {
            Some(dot) if dot > 0 && !path.is_dir() => dot,
            _ => name.len(),
        };
        let wrap = (self.tab.view.mode == ViewMode::Icons).then(|| theme::grid_label_width(self.tab.view.icon_size));
        let input = cx.new(|cx| TextInput::new(name, 0..stem, wrap, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::Changed => {}
            InputEvent::Submit => this.commit_rename(window, cx),
            InputEvent::Cancel => this.end_rename(window, cx),
        });
        window.focus(&input.focus_handle(cx));
        if let Some((row, _)) = self.visible_items(cx).iter().find(|(_, item)| *item == path) {
            self.reveal(*row);
        }
        self.tab.renaming = Some(Renaming { path, input, _subscription: subscription });
        cx.notify();
    }

    fn end_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tab.renaming = None;
        remove_ime_caret();
        window.focus(&self.focus);
        cx.notify();
    }

    /// 編集中の名前を確定する。編集していなければ何もしない。
    pub fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(renaming) = &self.tab.renaming else { return };
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
        self.tab.selection.select_only(item.with_file_name(&name));
        self.perform(Operation::Rename { item, name }, true, cx);
    }

    fn reveal(&mut self, row: usize) {
        match self.tab.view.mode {
            ViewMode::List => self.tab.list_scroll.scroll_to_item(row, ScrollStrategy::Top),
            ViewMode::Columns => self.tab.columns[self.tab.active_column].scroll.scroll_to_item(row, ScrollStrategy::Top),
            ViewMode::Icons => {}
        }
    }

    fn move_selection(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        let items = self.visible_items(cx);
        let ordered: Vec<PathBuf> = items.iter().map(|(_, path)| path.clone()).collect();
        let Some(path) = self.tab.selection.move_cursor(delta, extend, &ordered) else { return };
        if let Some((row, _)) = items.iter().find(|(_, item)| *item == path) {
            self.reveal(*row);
        }
        if self.tab.view.mode == ViewMode::Columns && !extend {
            self.follow_in_column(&path, cx);
        }
        cx.notify();
    }

    /// カラムで選択を動かした時、右の列も追従させる。
    fn follow_in_column(&mut self, path: &Path, cx: &mut Context<Self>) {
        let column = self.tab.active_column;
        let state = &self.tab.columns[column].pane.read(cx).state;
        let Some(name) = path.file_name() else { return };
        let Some(entry) = state.entries.iter().find(|entry| entry.os_name() == name).cloned() else { return };
        self.select_in_column(column, name.to_os_string(), entry, cx);
        self.tab.active_column = column;
    }

    fn step_column(&mut self, forward: bool, cx: &mut Context<Self>) {
        if forward {
            if self.tab.active_column + 1 < self.tab.columns.len() {
                self.tab.active_column += 1;
                self.tab.selection.clear();
                self.move_selection(1, false, cx);
            }
        } else if self.tab.active_column > 0 {
            self.tab.active_column -= 1;
            let column = &self.tab.columns[self.tab.active_column];
            let folder = column.pane.read(cx).state.path.clone();
            if let Some(name) = column.selected.clone() {
                self.tab.selection.select_only(folder.join(name));
            }
            self.tab.columns.truncate(self.tab.active_column + 2);
            cx.notify();
        }
    }

    pub fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.tab.selection.harden();
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        // 一覧にページ送りの操作は無いので、開いている間はクイックルックへ渡す
        let page = match keystroke.key.as_str() {
            "pageup" => -1,
            "pagedown" => 1,
            _ => 0,
        };
        if page != 0 && self.quick_look_page(page, cx) {
            return;
        }
        let chord = Chord::new(&keystroke.key, modifiers.control, modifiers.shift, modifiers.alt);
        let Some(command) = self.bindings.key(&chord) else { return };
        let typing = self.tab.renaming.is_some()
            || self.favorite_rename.is_some()
            || self.editing_path(window, cx)
            || self.tab.search_box.as_ref().is_some_and(|search| search.input.focus_handle(cx).is_focused(window));
        if (typing && !command.works_while_typing()) || (event.is_held && !command.repeats()) {
            return;
        }
        self.block_hover();
        if self.run_command(command, window, cx) {
            cx.stop_propagation();
        }
    }

    /// 操作を実行する。今の表示で意味が無い操作なら偽を返す。
    pub fn run_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mode = self.tab.view.mode;
        let rows = if mode == ViewMode::Icons { self.tab.grid_per_row as isize } else { 1 };
        match command {
            Command::QuickLook => return self.toggle_quick_look(window, cx),
            Command::FrameBack
            | Command::FrameForward
            | Command::FramesBack
            | Command::FramesForward
            | Command::SkipBack
            | Command::SkipForward
            | Command::Slower
            | Command::Faster
            | Command::ZoomIn
            | Command::ZoomOut
            | Command::ZoomToggle
            | Command::ZoomActual
            | Command::ZoomFit => return self.quick_look_command(command, cx),
            Command::Cancel if self.quick_look.is_some() => return self.close_quick_look(window, cx),
            Command::Parent => self.parent(cx),
            Command::Back => self.back(cx),
            Command::Forward => self.forward(cx),
            Command::MoveUp => self.move_selection(-rows, false, cx),
            Command::MoveDown => self.move_selection(rows, false, cx),
            Command::ExtendUp => self.move_selection(-rows, true, cx),
            Command::ExtendDown => self.move_selection(rows, true, cx),
            Command::MoveLeft | Command::ExtendLeft | Command::MoveRight | Command::ExtendRight => {
                let forward = matches!(command, Command::MoveRight | Command::ExtendRight);
                let extend = matches!(command, Command::ExtendLeft | Command::ExtendRight);
                match mode {
                    ViewMode::Icons => self.move_selection(if forward { 1 } else { -1 }, extend, cx),
                    ViewMode::Columns => self.step_column(forward, cx),
                    ViewMode::List => return false,
                }
            }
            Command::Open => self.open_selection(cx),
            Command::Cancel if self.tab.search_box.is_some() => self.drop_search(cx),
            Command::Cancel => {
                self.cut.clear();
                self.clear_selection(cx);
            }
            Command::Search => self.open_search(window, cx),
            Command::Recycle => self.recycle_selection(cx),
            Command::Rename => {
                if let [path] = self.tab.selection.paths().as_slice() {
                    self.begin_rename(path.clone(), window, cx);
                }
            }
            Command::Reload => self.tab.pane.update(cx, |pane, cx| pane.reload(cx)),
            Command::SelectAll => {
                let ordered = self.ordered_paths(cx);
                self.tab.selection.select_all(&ordered);
                cx.notify();
            }
            Command::Copy => self.copy_selection(false, cx),
            Command::Cut => self.copy_selection(true, cx),
            Command::Paste => self.paste(cx),
            Command::Undo => self.undo_last(cx),
            Command::NewFolder => self.new_folder(cx),
            Command::ToggleHidden => self.toggle_hidden(cx),
            Command::EditPath => self.open_path_edit(window, cx),
            Command::NewTab => self.new_tab(window, cx),
            Command::CloseTab => self.close_tab(self.tabs.index(), window, cx),
            Command::NextTab => self.select_tab(self.tabs.neighbor(true), window, cx),
            Command::PreviousTab => self.select_tab(self.tabs.neighbor(false), window, cx),
            Command::OpenInNewTab => {
                for path in self.tab.selection.paths() {
                    let is_dir = path.is_dir();
                    self.open_item_in_new_tab(path.clone(), is_dir, cx);
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{carries, same_volume};
    use std::path::{Path, PathBuf};

    #[test]
    fn a_folder_cannot_be_entered_while_it_is_dragged() {
        let dragged = [PathBuf::from(r"C:\a\b")];
        assert!(carries(&dragged, Path::new(r"C:\a\b")));
        assert!(carries(&dragged, Path::new(r"C:\a\b\c")));
        assert!(!carries(&dragged, Path::new(r"C:\a")));
        assert!(!carries(&dragged, Path::new(r"C:\a\bc")));
    }

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
        let entry = Entry { name: "a".into(), raw_name: None, is_dir: false, is_link: false, is_hidden: false, size: 0, modified: 0, created: 0 };
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

