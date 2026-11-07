//! タブ。フォルダの見え方に関わる状態を1枚ずつ持ち、ウィンドウは選んでいる1枚だけを描く。

use crate::actions::{NewItems, Renaming, SpringTarget, is_shortcut};
use crate::browser::{Browser, Column, TreeKey, saved_view};
use crate::folder_pane::FolderPane;
use crate::icons_view::GridData;
use crate::list_tree::Flattened;
use crate::scrollbar::{BarFade, ScrollTarget};
use crate::search::{SearchBox, SearchRun};
use crate::theme;
use filer_core::{Command, Entry, FolderView, Gesture, History, SavedTab, Selection, TabSession, ViewMode};
use filer_shell::{init_thread, remove_ime_caret, shortcut_folder};
use gpui::{
    AnyElement, App, Context, CursorStyle, DragMoveEvent, Entity, ExternalPaths, ListAlignment, ListState, MouseButton, ScrollHandle,
    ScrollWheelEvent, UniformListScrollHandle, Window, div, prelude::*, px, rgb,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct Tab {
    /// 裏の処理の結果を、頼んだタブへ返すための番号
    pub id: u64,
    /// 偽の間は一覧を読んでいない。最初に選ぶ時に読む
    pub loaded: bool,
    pub history: History,
    pub view: FolderView,
    pub pane: Entity<FolderPane>,
    pub selection: Selection,
    pub message: Option<String>,
    pub renaming: Option<Renaming>,
    pub search_box: Option<SearchBox>,
    pub search_run: Option<SearchRun>,
    pub pending_rename: Option<PathBuf>,
    pub rename_request: Option<PathBuf>,
    /// 右クリックの「新規作成」で増える項目を待っている
    pub new_items: Option<NewItems>,
    pub grid_per_row: usize,
    pub synced_version: u64,
    pub list_scroll: UniformListScrollHandle,
    pub list_columns_scroll: ScrollHandle,
    pub grid_list: ListState,
    pub grid_data: Option<GridData>,
    pub columns: Vec<Column>,
    pub columns_scroll: ScrollHandle,
    pub active_column: usize,
    /// ダブルクリックで開き、左端に寄せている列
    pub column_anchor: Option<usize>,
    /// 次の描画で、右端の列が見えるよう横へ送る
    pub columns_reveal: bool,
    pub bars: HashMap<ScrollTarget, BarFade>,
    pub tree_cache: Option<(TreeKey, Rc<Flattened>)>,
    pub preview: Option<(PathBuf, Entry)>,
    pub expanded: HashMap<PathBuf, Entity<FolderPane>>,
}

impl Tab {
    pub fn new(id: u64, path: PathBuf, view: FolderView, pane: Entity<FolderPane>, loaded: bool) -> Self {
        Self {
            id,
            loaded,
            history: History::new(path),
            view,
            pane,
            selection: Selection::default(),
            message: None,
            renaming: None,
            search_box: None,
            search_run: None,
            pending_rename: None,
            rename_request: None,
            new_items: None,
            grid_per_row: 1,
            synced_version: 0,
            list_scroll: UniformListScrollHandle::new(),
            list_columns_scroll: ScrollHandle::new(),
            grid_list: ListState::new(0, ListAlignment::Top, px(theme::ROW_HEIGHT * 10.)),
            grid_data: None,
            columns: Vec::new(),
            columns_scroll: ScrollHandle::new(),
            active_column: 0,
            column_anchor: None,
            columns_reveal: false,
            bars: HashMap::new(),
            tree_cache: None,
            preview: None,
            expanded: HashMap::new(),
        }
    }

    /// 操作の対象のフォルダ。カラム表示では操作中の列。
    pub fn folder(&self, cx: &App) -> PathBuf {
        match self.columns.get(self.active_column) {
            Some(column) if self.view.mode == ViewMode::Columns => column.pane.read(cx).state.path.clone(),
            _ => self.history.current().to_path_buf(),
        }
    }

    /// 検索の結果の表示を除いた、タブの表示形式。
    pub fn mode(&self) -> ViewMode {
        self.search_run.as_ref().map_or(self.view.mode, SearchRun::mode)
    }

    /// 読み込み済みの一覧すべて。検索の間は退避している分も含む。
    fn panes(&self) -> Vec<Entity<FolderPane>> {
        if !self.loaded {
            return Vec::new();
        }
        std::iter::once(&self.pane)
            .chain(self.columns.iter().map(|column| &column.pane))
            .chain(self.expanded.values())
            .chain(self.search_run.iter().flat_map(|run| run.panes()))
            .cloned()
            .collect()
    }
}

fn title_of(folder: &Path) -> String {
    folder.file_name().map_or_else(|| folder.display().to_string(), |name| name.to_string_lossy().into_owned())
}

impl Browser {
    /// 全タブの読み込み済みの一覧。
    pub fn all_panes(&self) -> Vec<Entity<FolderPane>> {
        self.tabs.iter(&self.tab).flat_map(Tab::panes).collect()
    }

    fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.position(&self.tab, |tab| tab.id == id)
    }

    pub fn tab_is_current(&self, id: u64) -> bool {
        self.tab.id == id
    }

    /// 読み込みを始めていないタブを作る。
    fn make_tab(&mut self, path: PathBuf, mode: ViewMode, cx: &mut Context<Self>) -> Tab {
        self.tab_ids += 1;
        let view = FolderView::for_folder(saved_view(&self.store, &path), mode);
        let (folder, order, show_hidden) = (path.clone(), view.order, self.settings.show_hidden);
        let pane = cx.new(|_| FolderPane::unopened(folder, order, show_hidden));
        cx.observe(&pane, |_, _, cx| cx.notify()).detach();
        Tab::new(self.tab_ids, path, view, pane, false)
    }

    /// 選んでいるタブが未読み込みなら読む。
    fn load_tab(&mut self, cx: &mut Context<Self>) {
        if !std::mem::replace(&mut self.tab.loaded, true) {
            let show_hidden = self.settings.show_hidden;
            self.tab.pane.update(cx, |pane, _| pane.state.show_hidden = show_hidden);
            self.show(self.tab.history.current().to_path_buf(), cx);
        }
    }

    /// `id` のタブを選んでいる形で `run` を動かし、選択を元に戻す。タブが閉じられていれば偽。
    pub fn on_tab(&mut self, id: u64, cx: &mut Context<Self>, run: impl FnOnce(&mut Self, &mut Context<Self>)) -> bool {
        let Some(target) = self.tab_index(id) else { return false };
        let home = self.tabs.index();
        self.tabs.select(target, &mut self.tab);
        run(self, cx);
        self.tabs.select(home, &mut self.tab);
        true
    }

    /// 新しいタブを今のタブの右に開く。今と同じフォルダと表示形式で始める。
    pub fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_rename(window, cx);
        let (folder, mode) = (self.current_folder(cx), self.tab.mode());
        let tab = self.make_tab(folder, mode, cx);
        self.leave_tab(cx);
        self.tabs.open_after(tab, &mut self.tab);
        self.load_tab(cx);
        cx.notify();
    }

    /// フォルダを裏のタブで開く。今のタブはそのまま。
    pub fn open_in_new_tab(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let tab = self.make_tab(path, self.tab.mode(), cx);
        let id = tab.id;
        self.tabs.insert_after(tab);
        self.on_tab(id, cx, Self::load_tab);
        if self.settings.focus_new_tabs {
            self.switch_to_hovered(id, cx);
        }
        cx.notify();
    }

    /// 項目を新しいタブで開く。ショートカットは行き先のフォルダを裏で調べてから開く。
    pub fn open_item_in_new_tab(&mut self, path: PathBuf, is_dir: bool, cx: &mut Context<Self>) {
        if is_dir {
            self.open_in_new_tab(path, cx);
            return;
        }
        if !is_shortcut(&path) {
            return;
        }
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            init_thread();
            let _ = tx.send(shortcut_folder(&path));
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Some(folder)) = rx.await {
                let _ = this.update(cx, |browser, cx| browser.open_in_new_tab(folder, cx));
            }
        })
        .detach();
    }

    /// マウスの呼び出し方に割り当てた操作を、項目に対して行う。
    pub fn item_gesture(&mut self, gesture: Gesture, path: PathBuf, is_dir: bool, cx: &mut Context<Self>) {
        if self.bindings.gesture(gesture) == Some(Command::OpenInNewTab) {
            self.open_item_in_new_tab(path, is_dir, cx);
        }
    }

    /// 帯の上のホイール。1目盛りで1枚動かす。
    fn wheel_tabs(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.wheel_switches_tabs {
            return;
        }
        self.tab_wheel += f32::from(event.delta.pixel_delta(px(theme::ROW_HEIGHT)).y);
        if self.tab_wheel.abs() >= theme::ROW_HEIGHT {
            let forward = self.tab_wheel < 0.;
            self.tab_wheel = 0.;
            self.select_tab(self.tabs.neighbor(forward), window, cx);
        }
    }

    /// `path` を表示している別のタブ。
    pub fn tab_showing(&self, path: &Path, cx: &App) -> Option<u64> {
        let key = |path: &Path| path.to_string_lossy().to_lowercase();
        let wanted = key(path);
        self.tabs.iter(&self.tab).find(|tab| tab.id != self.tab.id && key(&tab.folder(cx)) == wanted).map(|tab| tab.id)
    }

    /// `index` のタブを閉じる。最後の1枚ならウィンドウを閉じる。
    pub fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index == self.tabs.index() {
            self.commit_rename(window, cx);
        }
        if self.tabs.len() == 1 {
            window.remove_window();
            return;
        }
        if index == self.tabs.index() {
            self.leave_tab(cx);
        }
        self.tabs.close(index, &mut self.tab);
        self.load_tab(cx);
        cx.notify();
    }

    pub fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index != self.tabs.index() {
            self.commit_rename(window, cx);
            self.switch_tab(index, cx);
        }
    }

    /// ドラッグ中に止まったタブへ切り替える。
    pub fn switch_to_hovered(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.tab_index(id) {
            self.switch_tab(index, cx);
        }
    }

    fn switch_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.leave_tab(cx);
        self.tabs.select(index, &mut self.tab);
        self.load_tab(cx);
        cx.notify();
    }

    /// 今のタブを離れる前に、ウィンドウが持つ途中の操作を片付ける。
    fn leave_tab(&mut self, cx: &mut Context<Self>) {
        if self.tab.renaming.take().is_some() || self.tab.search_box.is_some() {
            remove_ime_caret();
        }
        if self.resizing.take().is_some() || self.icon_size_drag.take().is_some() {
            self.save_active_view(cx);
        }
        self.press = None;
        self.band = None;
        self.band_task = None;
        self.scroll_drag = None;
        self.header_drag = None;
        self.menu = None;
        self.spring = None;
        self.hovered_cell = None;
        self.suppress_click = false;
        self.grab_press = false;
        self.refocus = true;
    }

    /// タブの並びが変わっていれば保存する。パスを指定して開いたウィンドウは保存しない。
    pub fn save_session(&mut self, cx: &App) {
        if !self.keeps_session {
            return;
        }
        let Some(store) = &self.store else { return };
        let saved = |tab: &Tab| SavedTab { path: tab.folder(cx).to_string_lossy().into_owned(), mode: tab.mode() };
        let session = TabSession { tabs: self.tabs.iter(&self.tab).map(saved).collect(), active: self.tabs.index() };
        if self.saved_session.as_ref() == Some(&session) {
            return;
        }
        if let Err(error) = store.save_tab_session(&session) {
            eprintln!("タブを保存できませんでした: {error}");
        }
        self.saved_session = Some(session);
    }

    /// 前回のタブを、今のタブの左右へ足す。読み込むのは選ぶ時。
    pub fn restore_tabs(&mut self, left: Vec<SavedTab>, right: Vec<SavedTab>, cx: &mut Context<Self>) {
        for saved in left {
            let tab = self.make_tab(saved.path.into(), saved.mode, cx);
            self.tabs.insert_before(tab);
        }
        for saved in right.into_iter().rev() {
            let tab = self.make_tab(saved.path.into(), saved.mode, cx);
            self.tabs.insert_after(tab);
        }
    }

    fn tab_item(&self, tab: &Tab, active: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = tab.id;
        let close = div()
            .id(gpui::ElementId::Name(format!("tab-close-{id}").into()))
            .flex_none()
            .w(px(theme::TAB_CLOSE_WIDTH))
            .text_center()
            .rounded_sm()
            .text_color(rgb(theme::SUB_TEXT))
            .cursor(CursorStyle::PointingHand)
            .hover(|style| style.bg(rgb(theme::HOVER)))
            .child("×")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if let Some(index) = this.tab_index(id) {
                    this.close_tab(index, window, cx);
                }
            }));
        div()
            .id(gpui::ElementId::Name(format!("tab-{id}").into()))
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap_1()
            .px_1()
            .border_r_1()
            .border_color(rgb(theme::LINE))
            .bg(rgb(if self.flashing(&tab.folder(cx)) {
                theme::FLASH
            } else if active {
                theme::BACKGROUND
            } else {
                theme::BAR
            }))
            .when(!active, |item| item.cursor(CursorStyle::PointingHand))
            .when(!active && !self.flashing(&tab.folder(cx)), |item| item.hover(|style| style.bg(rgb(theme::HOVER))))
            .child(close)
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_center().child(title_of(&tab.folder(cx))))
            .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(rgb(theme::HOVER)))
            .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                this.hover_target(SpringTarget::Tab(id), event.bounds, event.event.position, cx);
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                cx.stop_propagation();
                let copy = filer_shell::control_down();
                let Some(index) = this.tab_index(id) else { return };
                let Some(tab) = this.tabs.iter(&this.tab).nth(index) else { return };
                let folder = tab.folder(cx);
                this.drop_paths(paths.paths(), folder, copy, cx);
            }))
            .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if let Some(index) = this.tab_index(id) {
                    this.close_tab(index, window, cx);
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(index) = this.tab_index(id) {
                    this.select_tab(index, window, cx);
                }
            }))
            .into_any_element()
    }

    /// ツールバーの下のタブの帯。
    pub fn tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let items: Vec<AnyElement> = self.tabs.iter(&self.tab).map(|tab| self.tab_item(tab, tab.id == self.tab.id, cx)).collect();
        div()
            .id("tab-bar")
            .flex()
            .flex_none()
            .items_center()
            .h(px(theme::TAB_BAR_HEIGHT))
            .bg(rgb(theme::BAR))
            .border_b_1()
            .border_color(rgb(theme::LINE))
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.commit_rename(window, cx)))
            .on_scroll_wheel(cx.listener(Self::wheel_tabs))
            .children(items)
            .child(
                div()
                    .id("tab-new")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(px(theme::TAB_NEW_WIDTH))
                    .h_full()
                    .cursor(CursorStyle::PointingHand)
                    .hover(|style| style.bg(rgb(theme::HOVER)))
                    .child("+")
                    .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
            )
    }
}
