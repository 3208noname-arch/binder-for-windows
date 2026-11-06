//! 選択、キー操作、ファイル操作など、画面の表示以外の動作。

use crate::browser::Browser;
use crate::icon_cache::SMALL;
use crate::icons_view::{GridRow, grid_metrics};
use crate::list_tree::{Flattened, Row, group_range};
use crate::text_input::{InputEvent, TextInput};
use crate::theme;
use filer_core::{DragHover, Entry, ViewMode, rows_in_band};
use filer_shell::{
    MenuChoice, Operation, Prompt, clipboard_files, cursor_position, drag_items, init_thread, left_button_down, open_path, remove_ime_caret, run_recording_undo,
    set_clipboard_files, show_folder_menu, show_items_menu, unique_folder_name, valid_file_name,
};
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, DragMoveEvent, Entity, ExternalPaths, Focusable, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, Point, ScrollStrategy, Stateful, Subscription, Task, Window, div, point, prelude::*, px, rgb, rgba,
};
use crate::folder_pane::FolderPane;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

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
    /// 名前の外を押した時は、ファイルを掴まず範囲選択を始める
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
    /// 小窓の見出し。その階層まで戻る
    PopupLevel(usize),
}

/// ドラッグ中に止まっている先。時間が来たら開く。
pub struct Spring {
    target: SpringTarget,
    _task: Task<()>,
}

/// ドラッグ中に開いているフォルダの小窓。
pub struct DragPopup {
    path: PathBuf,
    pane: Entity<FolderPane>,
    origin: Point<Pixels>,
    bounds: Option<Bounds<Pixels>>,
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

    /// 今読み込まれている項目のすべて。隠れている親の列も含む。
    fn known_paths(&self, cx: &Context<Self>) -> Vec<PathBuf> {
        let columns = self.columns.iter().map(|column| &column.pane);
        let panes = std::iter::once(&self.pane).chain(self.expanded.values()).chain(columns);
        let lists = panes.map(|pane| &pane.read(cx).state);
        lists.flat_map(|state| state.entries.iter().map(|entry| state.path.join(entry.os_name()))).collect()
    }

    /// 消えた項目や、閉じたフォルダの中の項目を選択から外す。
    /// 新規フォルダの一覧への反映を待って、名前の編集を始める。
    pub fn sync_with_listing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.pane.read(cx);
        let version = self.expanded.values().map(|pane| pane.read(cx).version).sum::<u64>()
            + self.columns.iter().map(|column| column.pane.read(cx).version).sum::<u64>()
            + root.version
            + self.expanded.len() as u64;
        if version == self.synced_version || root.state.loading {
            return;
        }
        self.synced_version = version;
        let known = self.known_paths(cx);
        self.selection.retain_existing(&known);
        if let Some(path) = self.pending_rename.take_if(|path| known.contains(path)) {
            self.selection.select_only(path.clone());
            self.begin_rename(path, window, cx);
        }
    }

    fn ordered_paths(&mut self, cx: &Context<Self>) -> Vec<PathBuf> {
        self.visible_items(cx).into_iter().map(|(_, path)| path).collect()
    }

    /// クリックでの選択。Ctrlは追加、Shiftは範囲。ダブルクリックは開く。
    pub fn click_item(&mut self, path: PathBuf, is_dir: bool, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.suppress_click) {
            return;
        }
        if self.renaming.as_ref().is_some_and(|renaming| renaming.path == path) {
            return;
        }
        self.soft_selection = false;
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
    ///
    /// `name_span` は名前のある横の範囲。その外を押すと範囲選択になる。
    pub fn item_mouse(&self, row: Stateful<gpui::Div>, path: PathBuf, is_dir: bool, name_span: Option<(f32, f32, Option<usize>)>, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let open_in_popup = self.drag_popups.iter().any(|popup| popup.path == path);
        let (press_path, menu_path, drop_path) = (path.clone(), path.clone(), path);
        row.when(open_in_popup, |row| row.bg(rgb(theme::HOVER))).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if this.renaming.as_ref().is_some_and(|renaming| renaming.path == press_path) {
                    return;
                }
                this.commit_rename(window, cx);
                this.menu = None;
                this.suppress_click = false;
                let x = f32::from(event.position.x);
                let band_column = name_span.and_then(|(left, right, column)| (x < left || x > right).then_some(column));
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
            let spring_path = drop_path.clone();
            row.drag_over::<ExternalPaths>(|style, _, _, _| style.bg(rgb(theme::HOVER)))
                .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                    let target = SpringTarget::Folder(spring_path.clone());
                    this.hover_target(target, event.bounds, event.event.position, cx);
                }))
                .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                    cx.stop_propagation();
                    let copy = window.modifiers().control;
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
            self.selection.clear();
            self.begin_band(press.start, column, event.modifiers.control, cx);
            return;
        }
        if !self.selection.contains(&press.path) {
            self.selection.select_only(press.path.clone());
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
    fn run_menu(cx: &mut Context<Self>, rename: Option<PathBuf>, modal: impl FnOnce() -> Result<MenuChoice, windows::core::Error> + 'static) {
        cx.spawn(async move |this, cx| {
            match modal() {
                Ok(MenuChoice::Rename) => {
                    let _ = this.update(cx, |browser, cx| {
                        browser.rename_request = rename;
                        cx.notify();
                    });
                }
                Ok(_) => {}
                Err(error) => eprintln!("メニューを出せませんでした: {error}"),
            }
        })
        .detach();
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
        Self::run_menu(cx, Some(path), move || show_items_menu(hwnd, &folder, &names));
    }

    /// 余白の右クリック。新規作成や貼り付けのメニュー。
    pub fn folder_menu(&mut self, cx: &mut Context<Self>) {
        let (hwnd, folder) = (self.hwnd, self.current_folder(cx));
        Self::run_menu(cx, None, move || show_folder_menu(hwnd, &folder));
    }

    /// エクスプローラーと同じく、同じドライブなら移動、別なら複写にする。Ctrlは複写。
    pub fn drop_paths(&mut self, paths: &[PathBuf], target: PathBuf, copy: bool, cx: &mut Context<Self>) {
        self.end_spring();
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
            let _ = tx.send(run_recording_undo(&job, Prompt::Windows { hwnd }));
        });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err("ファイル操作が途中で止まりました".to_string()));
            let _ = this.update(cx, |browser, cx| browser.finish_operation(&operation, result, record_undo, cx));
        })
        .detach();
    }

    fn finish_operation(&mut self, operation: &Operation, result: Result<Option<Operation>, String>, record_undo: bool, cx: &mut Context<Self>) {
        match result {
            Ok(undo) => {
                match undo {
                    Some(undo) if record_undo => self.undo.push(undo),
                    // 戻せない操作の後で、さらに前の操作を戻すと順番が崩れる
                    None if record_undo => self.undo.clear(),
                    _ => {}
                }
                if let Operation::NewFolder { parent, name } = operation {
                    self.pending_rename = Some(parent.join(name));
                }
                if matches!(operation, Operation::Move { .. }) {
                    self.cut.clear();
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
        let items = self.selection.top_level();
        match set_clipboard_files(&items, cut) {
            Ok(()) => self.cut = if cut { items.into_iter().collect() } else { HashSet::new() },
            Err(message) => self.message = Some(message),
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
            self.active_column = column;
        }
        let tree = self.active_tree(cx);
        let range = group_range(&tree, ordinal);
        let rows = self.visible_items(cx).into_iter();
        let members: Vec<PathBuf> = rows.filter(|(ix, _)| range.contains(ix)).map(|(_, path)| path).collect();
        self.selection.select_all(&members);
        window.focus(&self.focus);
        cx.notify();
    }

    fn active_tree(&mut self, cx: &Context<Self>) -> Rc<Flattened> {
        match self.view.mode {
            ViewMode::List => self.tree(cx),
            ViewMode::Columns => self.column_tree(self.active_column, cx),
            ViewMode::Icons => Rc::new(Self::plain_tree(self.pane.read(cx), self.view.group).1),
        }
    }

    /// ドラッグ中に、開く先の上で止まったら少し待って開く。
    pub fn hover_target(&mut self, target: SpringTarget, bounds: Bounds<Pixels>, position: Point<Pixels>, cx: &mut Context<Self>) {
        let inside = bounds.contains(&position);
        if let SpringTarget::Folder(path) = &target
            && self.drag_popups.iter().any(|popup| popup.path == *path)
        {
            return;
        }
        let current = self.spring.as_ref().is_some_and(|spring| spring.target == target);
        if inside == current {
            return;
        }
        if !inside {
            self.spring = None;
            return;
        }
        let delay = self.spring_delay(&target);
        let fired = target.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(delay)).await;
            let _ = this.update(cx, |browser, cx| browser.open_hovered(fired, bounds, position, cx));
        });
        self.spring = Some(Spring { target, _task: task });
    }

    /// 開くまでの待ち時間。開き方ごとに設定できる。
    fn spring_delay(&self, target: &SpringTarget) -> u64 {
        let open = if self.springs_done == 0 { self.settings.spring_first_ms } else { self.settings.spring_next_ms };
        match target {
            SpringTarget::PopupLevel(_) => theme::POPUP_RETURN_DELAY_MS,
            SpringTarget::Folder(_) if self.view.mode == ViewMode::Columns => self.settings.spring_column_ms,
            SpringTarget::Folder(_) if self.settings.drag_hover == DragHover::Popup => self.settings.spring_popup_ms,
            _ => open,
        }
    }

    fn open_hovered(&mut self, target: SpringTarget, anchor: Bounds<Pixels>, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.spring = None;
        if !cx.has_active_drag() {
            return;
        }
        self.springs_done += 1;
        match target {
            SpringTarget::Back => self.back(cx),
            SpringTarget::Parent => self.parent(cx),
            SpringTarget::Go(path) => self.navigate(path, cx),
            SpringTarget::PopupLevel(level) => self.drag_popups.truncate(level + 1),
            SpringTarget::Folder(path) => match (self.view.mode, self.settings.drag_hover) {
                (ViewMode::Columns, _) => self.open_column_for(&path, cx),
                (_, DragHover::Popup) => self.open_popup(path, anchor, position, cx),
                (ViewMode::List, DragHover::Open) => {
                    if !self.expanded.contains_key(&path) {
                        let pane = self.new_pane(path.clone(), self.view.order, false, cx);
                        self.expanded.insert(path, pane);
                    }
                }
                (ViewMode::Icons, DragHover::Open) => self.navigate(path, cx),
            },
        }
        cx.notify();
    }

    /// 小窓を開く。小窓の中のフォルダなら親の右に開き、入らない時は親に重ねてずらす。
    fn open_popup(&mut self, path: PathBuf, anchor: Bounds<Pixels>, position: Point<Pixels>, cx: &mut Context<Self>) {
        let parent = self.drag_popups.iter().position(|popup| Some(popup.path.as_path()) == path.parent());
        self.drag_popups.truncate(parent.map_or(0, |index| index + 1));
        let width = px(theme::DRAG_POPUP_WIDTH);
        let limit = self.viewport.width - width - px(theme::DRAG_POPUP_GAP * 2.);
        let origin = match parent {
            Some(index) => {
                let above = &self.drag_popups[index];
                let right = above.bounds.map_or(above.origin.x + width, |bounds| bounds.right());
                let beside = right + px(theme::DRAG_POPUP_GAP);
                if beside > limit {
                    // 親の見出しが見えるよう、下へずらして重ねる
                    point(limit.min(above.origin.x + px(theme::POPUP_CASCADE)), above.origin.y + px(theme::POPUP_CASCADE))
                } else {
                    point(beside, anchor.top())
                }
            }
            None => point(limit.min(position.x + px(theme::DRAG_POPUP_OFFSET)), position.y + px(theme::DRAG_POPUP_OFFSET)),
        };
        let lowest = self.viewport.height - px(theme::POPUP_ROW_ESTIMATE * 4.);
        let origin = point(origin.x, origin.y.min(lowest));
        let pane = self.new_pane(path.clone(), self.view.order, false, cx);
        self.drag_popups.push(DragPopup { path, pane, origin, bounds: None });
    }

    fn open_column_for(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(parent) = path.parent() else { return };
        let Some(name) = path.file_name() else { return };
        let found = self.columns.iter().position(|column| column.pane.read(cx).state.path == parent);
        let Some(column) = found else { return };
        let state = &self.columns[column].pane.read(cx).state;
        let Some(entry) = state.entries.iter().find(|entry| entry.os_name() == name).cloned() else { return };
        self.select_in_column(column, name.to_os_string(), entry, cx);
    }

    /// ドラッグの終わりに、待っている開閉と小窓を片付ける。
    pub fn end_spring(&mut self) {
        self.spring = None;
        self.springs_done = 0;
        self.drag_popups.clear();
    }

    fn popup_panel(&mut self, level: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let popup = &self.drag_popups[level];
        let (folder, origin) = (popup.path.clone(), popup.origin);
        let entries: Vec<Entry> = popup.pane.read(cx).state.entries.clone();
        let more = entries.len().saturating_sub(theme::DRAG_POPUP_ROWS);
        let rows: Vec<_> = entries
            .iter()
            .take(theme::DRAG_POPUP_ROWS)
            .enumerate()
            .map(|(ix, entry)| {
                let path = folder.join(entry.os_name());
                let icon = self.icons.update(cx, |icons, _| icons.get(&folder, entry, SMALL));
                let row = div()
                    .id(ix)
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(theme::ROW_HEIGHT))
                    .px_2()
                    .child(self.icon(icon, entry.is_dir, theme::ROW_ICON))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(entry.name.clone()));
                self.item_mouse(row, path, entry.is_dir, None, cx)
            })
            .collect();
        let title = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let target = folder.clone();
        div()
            .id(("drag-popup", level))
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .w(px(theme::DRAG_POPUP_WIDTH))
            .py_1()
            .rounded_md()
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::LINE))
            .shadow_md()
            .occlude()
            .child(
                div()
                    .id("popup-title")
                    .px_2()
                    .pb_1()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title)
                    .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                        // 下の階層を開いている時だけ、見出しの上で戻る
                        if level + 1 < this.drag_popups.len() {
                            this.hover_target(SpringTarget::PopupLevel(level), event.bounds, event.event.position, cx);
                        }
                    })),
            )
            .children(rows)
            .when(more > 0, |panel| panel.child(div().px_2().text_color(rgb(theme::SUB_TEXT)).child(format!("ほか {more} 項目"))))
            .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, _| {
                if let Some(popup) = this.drag_popups.get_mut(level) {
                    popup.bounds = Some(event.bounds);
                }
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                cx.stop_propagation();
                let copy = window.modifiers().control;
                this.drop_paths(paths.paths(), target.clone(), copy, cx);
            }))
    }

    /// ドラッグ中に出すフォルダの中身の小窓。
    pub fn drag_popup_views(&mut self, cx: &mut Context<Self>) -> Vec<impl IntoElement + use<>> {
        if !self.drag_popups.is_empty() && !cx.has_active_drag() {
            self.end_spring();
        }
        (0..self.drag_popups.len()).map(|level| self.popup_panel(level, cx)).collect()
    }

    /// 余白を押した時に、範囲選択を始める。カーソルはウィンドウの外でも追う。
    pub fn begin_band(&mut self, position: Point<Pixels>, column: Option<usize>, control: bool, cx: &mut Context<Self>) {
        let Some(view) = self.band_viewport(column) else { return };
        let start = point(position.x - view.bounds.origin.x, position.y - view.bounds.origin.y - view.scroll);
        let base = if control { self.selection.paths() } else { Vec::new() };
        self.band = Some(Band { start, current: position, column, base });
        self.soft_selection = true;
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
        let (bounds, scroll) = match self.view.mode {
            ViewMode::List => {
                let handle = self.list_scroll.0.borrow().base_handle.clone();
                (handle.bounds(), handle.offset().y)
            }
            ViewMode::Columns => {
                let handle = self.columns.get(column?)?.scroll.0.borrow().base_handle.clone();
                (handle.bounds(), handle.offset().y)
            }
            ViewMode::Icons => (self.grid_list.viewport_bounds(), self.grid_list.scroll_px_offset_for_scrollbar().y),
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
        let hits = match self.view.mode {
            ViewMode::Icons => self.grid_hits(start, end, cx),
            ViewMode::List | ViewMode::Columns => {
                if let Some(column) = column {
                    self.active_column = column;
                }
                let tree = self.active_tree(cx);
                let rows = rows_in_band(start.y.into(), end.y.into(), theme::ROW_HEIGHT, tree.rows.len());
                let items = self.visible_items(cx).into_iter();
                items.filter(|(ix, _)| rows.contains(ix)).map(|(_, path)| path).collect()
            }
        };
        self.selection.select_all(&[base, hits].concat());
        cx.notify();
        true
    }

    /// カーソルが一覧の上下へ出ている間は、離れた距離に応じてスクロールする。
    fn scroll_with_band(&self, column: Option<usize>, cursor: Point<Pixels>, bounds: Bounds<Pixels>) {
        let (y, top, bottom): (f32, f32, f32) = (cursor.y.into(), bounds.top().into(), bounds.bottom().into());
        let beyond = if y > bottom { y - bottom } else if y < top { y - top } else { return };
        let step = (beyond.abs().min(theme::BAND_SCROLL_LIMIT) * theme::BAND_SCROLL_SPEED).max(1.0) * beyond.signum();
        let handle = match self.view.mode {
            ViewMode::Icons => {
                self.grid_list.scroll_by(px(step));
                return;
            }
            ViewMode::List => self.list_scroll.0.borrow().base_handle.clone(),
            ViewMode::Columns => match column.and_then(|column| self.columns.get(column)) {
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
        let Some(data) = &self.grid_data else { return Vec::new() };
        let (cell_width, row_height) = grid_metrics(self.view.icon_size);
        let (left, right): (f32, f32) = (start.x.min(end.x).into(), start.x.max(end.x).into());
        let (top, bottom): (f32, f32) = (start.y.min(end.y).into(), start.y.max(end.y).into());
        let state = &self.pane.read(cx).state;
        let mut hits = Vec::new();
        let mut y = 0.0;
        for row in data.rows.iter() {
            match row {
                GridRow::Header { .. } => y += theme::ROW_HEIGHT,
                GridRow::Cells(cells) => {
                    if y < bottom && y + row_height > top {
                        for (index, &entry) in cells.iter().enumerate() {
                            let x = theme::GRID_SIDE_PADDING + index as f32 * (cell_width + theme::GRID_CELL_GAP);
                            if x < right && x + cell_width > left
                                && let Some(entry) = state.entries.get(entry as usize)
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
        let current = self.history.current().to_path_buf();
        let mut ancestors: Vec<PathBuf> = current.ancestors().skip(1).take(theme::DRAG_BAR_SEGMENTS).map(PathBuf::from).collect();
        ancestors.reverse();
        let back = self.history.can_go_back().then(|| Self::drag_segment("drag-back".into(), "‹ 戻る".into(), SpringTarget::Back, None, cx));
        let segments: Vec<_> = ancestors
            .into_iter()
            .map(|path| {
                let label = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                let id = format!("drag-go-{}", path.display());
                Self::drag_segment(id, label, SpringTarget::Go(path.clone()), Some(path), cx)
            })
            .collect();
        div()
            .absolute()
            .left(px(theme::SIDEBAR_WIDTH))
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
            .children(segments)
    }

    fn drag_segment(id: String, label: String, target: SpringTarget, drop: Option<PathBuf>, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(gpui::ElementId::Name(id.into()))
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_center()
            .h(px(theme::DRAG_BAR_HEIGHT - 12.))
            .px_3()
            .rounded_md()
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::LINE))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(rgb(theme::HOVER)))
            .child(label)
            .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                this.hover_target(target.clone(), event.bounds, event.event.position, cx);
            }))
            .when_some(drop, |segment, path| {
                segment.on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                    cx.stop_propagation();
                    let copy = window.modifiers().control;
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
        if !self.selection.is_empty() {
            let items = self.selection.top_level();
            self.selection.clear();
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
        let input = cx.new(|cx| TextInput::new(name, 0..stem, cx));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::Submit => this.commit_rename(window, cx),
            InputEvent::Cancel => this.end_rename(window, cx),
        });
        window.focus(&input.focus_handle(cx));
        if let Some((row, _)) = self.visible_items(cx).iter().find(|(_, item)| *item == path) {
            self.reveal(*row);
        }
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
        self.soft_selection = false;
        if self.renaming.is_some() {
            return;
        }
        let keystroke = &event.keystroke;
        // 押しっぱなしで繰り返すのは移動だけにする
        if event.is_held && !matches!(keystroke.key.as_str(), "up" | "down" | "left" | "right") {
            return;
        }
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
            "escape" => {
                self.cut.clear();
                self.clear_selection(cx);
            }
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

