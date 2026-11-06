use crate::actions::{Renaming, RowPress};
use crate::folder_pane::{FolderPane, tree_for};
use crate::icon_cache::IconCache;
use crate::icons_view::GridData;
use crate::list_tree::Flattened;
use crate::size_cache::SizeCache;
use crate::theme;
use filer_core::{Date, Entry, Field, FolderView, GroupKey, History, Selection, SortKey, SortOrder, ViewMode};
use filer_shell::{Operation, drives, favorites, today};
use filer_store::Store;
use gpui::{
    AnyElement, Context, CursorStyle, Entity, ExternalPaths, FocusHandle, Image, ListAlignment, ListState, MouseButton,
    MouseMoveEvent, Pixels, Point, ScrollHandle, UniformListScrollHandle, Window, div, img, prelude::*, px, rgb,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

pub struct Column {
    pub pane: Entity<FolderPane>,
    pub scroll: UniformListScrollHandle,
    pub selected: Option<OsString>,
    pub view: FolderView,
    pub tree: Option<(TreeKey, Rc<Flattened>)>,
}

/// 幅をドラッグで変えている最中の状態。
#[derive(Clone, Copy)]
pub enum Resize {
    Column { column: usize, start_x: f32, start_width: f32 },
    Field { index: usize, start_x: f32, start_width: f32 },
}

/// リストの見出しを押している最中の状態。動かさず離せば並べ替えになる。
#[derive(Clone, Copy)]
pub struct HeaderDrag {
    pub index: usize,
    pub start_x: f32,
    pub target: Option<usize>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Menu {
    Sort,
    Group,
    Fields(Point<Pixels>),
}

/// 利用者が選べる設定。設定画面は後で足す。
pub struct Settings {
    /// アイコン表示の選択を、マウスオーバーと同じくセル全体で示す
    pub grid_selection_matches_hover: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { grid_selection_matches_hover: true }
    }
}

/// 行の並びが古くなっていないかを判断する材料。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TreeKey {
    pub root_version: u64,
    pub expanded_version: u64,
    pub expanded_count: usize,
    pub group: GroupKey,
    pub today: Date,
}

/// ウィンドウ全体。履歴と表示の設定を持ち、3つの表示へ分ける。
pub struct Browser {
    pub history: History,
    pub view: FolderView,
    pub store: Option<Store>,
    pub pane: Entity<FolderPane>,
    pub selection: Selection,
    pub hwnd: isize,
    pub focus: FocusHandle,
    pub message: Option<String>,
    pub renaming: Option<Renaming>,
    pub pending_rename: Option<PathBuf>,
    pub press: Option<RowPress>,
    pub undo: Vec<Operation>,
    pub grid_per_row: usize,
    pub synced_version: u64,
    pub list_scroll: UniformListScrollHandle,
    pub list_columns_scroll: ScrollHandle,
    pub grid_list: ListState,
    pub grid_data: Option<GridData>,
    pub columns: Vec<Column>,
    pub columns_scroll: ScrollHandle,
    pub active_column: usize,
    pub resizing: Option<Resize>,
    pub header_drag: Option<HeaderDrag>,
    pub settings: Settings,
    pub menu: Option<Menu>,
    pub tree_cache: Option<(TreeKey, Rc<Flattened>)>,
    pub window_active: bool,
    pub preview: Option<(PathBuf, Entry)>,
    pub expanded: HashMap<PathBuf, Entity<FolderPane>>,
    pub icons: Entity<IconCache>,
    pub sizes: Entity<SizeCache>,
    favorites: Vec<(String, PathBuf)>,
    drives: Vec<PathBuf>,
}

impl Browser {
    pub fn new(
        path: PathBuf,
        mode: Option<ViewMode>,
        store: Option<Store>,
        measure: Option<Instant>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_window_activation(window, |_, _, cx| cx.notify()).detach();
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut view = FolderView::for_folder(saved_view(&store, &path), ViewMode::List);
        if let Some(mode) = mode {
            view.mode = mode;
        }
        let (order, wants) = (view.order, view.needs_properties());
        let pane = cx.new(|cx| {
            let mut pane = FolderPane::new(path.clone(), order, cx);
            pane.set_wants_properties(wants, cx);
            if let Some(start) = measure {
                pane.measure_from(start);
            }
            pane
        });
        cx.observe(&pane, |_, _, cx| cx.notify()).detach();
        let icons = cx.new(IconCache::new);
        cx.observe(&icons, |_, _, cx| cx.notify()).detach();
        let sizes = cx.new(SizeCache::new);
        cx.observe(&sizes, |_, _, cx| cx.notify()).detach();
        let mut browser = Self {
            history: History::new(path),
            view,
            store,
            pane,
            selection: Selection::default(),
            hwnd: window_handle(window),
            focus: focus.clone(),
            message: None,
            renaming: None,
            pending_rename: None,
            press: None,
            undo: Vec::new(),
            grid_per_row: 1,
            synced_version: 0,
            list_scroll: UniformListScrollHandle::new(),
            list_columns_scroll: ScrollHandle::new(),
            grid_list: ListState::new(0, ListAlignment::Top, px(theme::GRID_ROW_HEIGHT * 2.)),
            grid_data: None,
            columns: Vec::new(),
            columns_scroll: ScrollHandle::new(),
            active_column: 0,
            resizing: None,
            header_drag: None,
            settings: Settings::default(),
            menu: None,
            tree_cache: None,
            window_active: true,
            preview: None,
            expanded: HashMap::new(),
            icons,
            sizes,
            favorites: favorites(),
            drives: drives(),
        };
        browser.reset_columns();
        browser
    }

    pub fn new_pane(&self, path: PathBuf, order: SortOrder, wants: bool, cx: &mut Context<Self>) -> Entity<FolderPane> {
        let pane = cx.new(|cx| {
            let mut pane = FolderPane::new(path, order, cx);
            pane.set_wants_properties(wants, cx);
            pane
        });
        cx.observe(&pane, |_, _, cx| cx.notify()).detach();
        pane
    }

    /// 開くフォルダ用に、そのフォルダの保存済みの設定を読む。
    pub fn view_of(&self, path: &Path) -> FolderView {
        FolderView::for_folder(saved_view(&self.store, path), self.view.mode)
    }

    fn reset_columns(&mut self) {
        self.columns = vec![Column {
            pane: self.pane.clone(),
            scroll: UniformListScrollHandle::new(),
            selected: None,
            view: self.view.clone(),
            tree: None,
        }];
        self.active_column = 0;
        self.preview = None;
    }

    /// 履歴を増やして移動する。
    pub fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.history.visit(path.clone());
        self.show(path, cx);
    }

    fn show(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.selection.clear();
        self.message = None;
        self.menu = None;
        self.renaming = None;
        self.expanded.clear();
        self.sizes.update(cx, |sizes, _| sizes.cancel_running());
        self.list_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.grid_list.reset(0);
        self.grid_data = None;
        self.view = self.view_of(&path);
        let (order, wants) = (self.view.order, self.view.needs_properties());
        self.pane.update(cx, |pane, cx| {
            pane.state.order = order;
            pane.set_wants_properties(wants, cx);
            pane.open(path, cx);
        });
        self.reset_columns();
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.history.go_back().map(PathBuf::from) {
            self.show(path, cx);
        }
    }

    pub fn forward(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.history.go_forward().map(PathBuf::from) {
            self.show(path, cx);
        }
    }

    pub fn parent(&mut self, cx: &mut Context<Self>) {
        if let Some(parent) = self.history.current().parent().map(PathBuf::from) {
            self.navigate(parent, cx);
        }
    }

    /// 開いているフォルダの設定を変え、すぐ保存する。カラム表示では操作中の列が対象。
    pub fn update_view(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut FolderView)) {
        let in_child_column = self.view.mode == ViewMode::Columns && self.active_column > 0;
        let (order, wants, pane) = if in_child_column {
            let column = &mut self.columns[self.active_column];
            change(&mut column.view);
            (column.view.order, column.view.needs_properties(), column.pane.clone())
        } else {
            change(&mut self.view);
            self.columns[0].view = self.view.clone();
            (self.view.order, self.view.needs_properties(), self.pane.clone())
        };
        pane.update(cx, |pane, cx| {
            pane.set_order(order, cx);
            pane.set_wants_properties(wants, cx);
        });
        if !in_child_column {
            for child in self.expanded.values() {
                child.update(cx, |pane, cx| pane.set_order(order, cx));
            }
        }
        self.save_active_view(cx);
        cx.notify();
    }

    /// 幅のドラッグの終わりなど、値を変えた後で保存する。
    pub fn save_active_view(&self, cx: &Context<Self>) {
        let Some(store) = &self.store else { return };
        let in_child_column = self.view.mode == ViewMode::Columns && self.active_column > 0;
        let result = if in_child_column {
            let column = &self.columns[self.active_column];
            store.save_folder_view(&column.pane.read(cx).state.path, &column.view)
        } else {
            store.save_folder_view(self.history.current(), &self.view)
        };
        if let Err(error) = result {
            eprintln!("表示の設定を保存できませんでした: {error}");
        }
    }

    pub fn toggle_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.update_view(cx, |view| {
            let descending = view.order.key == key && !view.order.descending;
            view.order = SortOrder { key, descending };
        });
    }

    fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.active_column = 0;
        self.update_view(cx, |view| view.mode = mode);
    }

    /// 並べ替えとグループの対象になっているフォルダの設定。
    fn active_view(&self) -> &FolderView {
        match self.view.mode {
            ViewMode::Columns if self.active_column > 0 => &self.columns[self.active_column].view,
            _ => &self.view,
        }
    }

    /// 選択の色。別のウィンドウへ移った時は灰色にする。
    pub fn selection_colors(&self) -> (u32, u32) {
        if self.window_active {
            (theme::SELECTED, theme::SELECTED_TEXT)
        } else {
            (theme::SELECTED_INACTIVE, theme::TEXT)
        }
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        self.menu = None;
        cx.notify();
    }

    /// フォルダの行の並び。開いている子フォルダは含まない。
    pub fn plain_tree(pane: &FolderPane, group: GroupKey) -> (TreeKey, Flattened) {
        let today = today();
        let key = TreeKey { root_version: pane.version, expanded_version: 0, expanded_count: 0, group, today };
        let lists = HashMap::from([(pane.state.path.clone(), pane.state.entries.as_slice())]);
        (key, tree_for(pane, &lists, group, today))
    }

    fn drag(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        // ウィンドウの外でボタンを離した時も、変えた幅は残す
        if event.pressed_button != Some(MouseButton::Left) {
            self.press = None;
            if self.resizing.take().is_some() {
                self.save_active_view(cx);
            }
            self.header_drag = None;
            return;
        }
        if self.press.is_some() {
            self.drag_row(event, cx);
        } else if self.resizing.is_some() {
            self.drag_resize(event, cx);
        } else if self.header_drag.is_some() {
            self.drag_header(event, window, cx);
        }
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        self.end_press();
        if self.resizing.take().is_some() {
            self.save_active_view(cx);
        }
        if let Some(drag) = self.header_drag.take() {
            self.drop_header(drag, cx);
        }
    }

    /// 行のアイコン。届く前は仮の図形を出す。
    pub fn icon(&self, icon: Option<Arc<Image>>, is_dir: bool, size: f32) -> AnyElement {
        match icon {
            Some(image) => img(image).size(px(size)).flex_none().into_any_element(),
            None => {
                let color = if is_dir { theme::FOLDER } else { theme::FILE };
                div().flex_none().size(px(size)).rounded_sm().bg(rgb(color)).into_any_element()
            }
        }
    }

    fn tool_button(id: &'static str, label: &'static str, enabled: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px_2()
            .rounded_md()
            .text_color(rgb(if enabled { theme::TEXT } else { theme::DISABLED_TEXT }))
            .when(enabled, |b| b.cursor(CursorStyle::PointingHand).hover(|s| s.bg(rgb(theme::HOVER))))
            .child(label)
    }

    fn segment(&self, id: &'static str, label: &'static str, mode: ViewMode, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.view.mode == mode;
        div()
            .id(id)
            .px_2()
            .rounded_sm()
            .cursor(CursorStyle::PointingHand)
            .when(active, |b| b.bg(rgb(theme::BACKGROUND)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx)))
    }

    fn menu_button(&self, id: &'static str, label: String, menu: Menu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id(id)
            .flex_none()
            .px_2()
            .rounded_md()
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme::HOVER)))
            .child(label)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.menu = if this.menu == Some(menu) { None } else { Some(menu) };
                cx.notify();
            }))
    }

    fn popup(position: impl FnOnce(gpui::Div) -> gpui::Div, items: Vec<AnyElement>) -> impl IntoElement {
        position(div().absolute())
            .w(px(theme::GROUP_MENU_WIDTH))
            .py_1()
            .rounded_md()
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::LINE))
            .shadow_md()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(items)
    }

    fn menu_item(id: String, label: String, checked: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(gpui::ElementId::Name(id.into()))
            .flex()
            .gap_2()
            .px_3()
            .py_0p5()
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme::HOVER)))
            .child(div().w(px(12.)).child(if checked { "✓" } else { "" }))
            .child(label)
    }

    fn sort_menu(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let order = self.active_view().order;
        let mut items: Vec<AnyElement> = SortKey::ALL
            .iter()
            .map(|&key| {
                Self::menu_item(format!("sort-{key:?}"), key.label().into(), order.key == key)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        this.update_view(cx, |view| view.order.key = key);
                    }))
                    .into_any_element()
            })
            .collect();
        items.push(div().my_1().h(px(1.)).bg(rgb(theme::LINE)).into_any_element());
        for (id, label, descending) in [("asc", "昇順", false), ("desc", "降順", true)] {
            items.push(
                Self::menu_item(id.into(), label.into(), order.descending == descending)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        this.update_view(cx, |view| view.order.descending = descending);
                    }))
                    .into_any_element(),
            );
        }
        items
    }

    fn group_menu(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = self.active_view().group;
        GroupKey::ALL
            .iter()
            .map(|&key| {
                Self::menu_item(format!("group-{key:?}"), key.label().into(), key == current)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.menu = None;
                        this.update_view(cx, |view| view.group = key);
                    }))
                    .into_any_element()
            })
            .collect()
    }

    fn fields_menu(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        Field::ALL
            .iter()
            .map(|&field| {
                Self::menu_item(format!("field-{field:?}"), field.label().into(), self.view.has_field(field))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_view(cx, |view| view.toggle_field(field));
                    }))
                    .into_any_element()
            })
            .collect()
    }

    fn open_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let top = px(theme::TOOLBAR_HEIGHT);
        Some(match self.menu? {
            Menu::Sort => {
                let items = self.sort_menu(cx);
                Self::popup(|d| d.top(top).right(px(theme::MENU_RIGHT_SORT)), items).into_any_element()
            }
            Menu::Group => {
                let items = self.group_menu(cx);
                Self::popup(|d| d.top(top).right(px(theme::GROUP_MENU_RIGHT)), items).into_any_element()
            }
            Menu::Fields(position) => {
                let items = self.fields_menu(cx);
                Self::popup(|d| d.left(position.x).top(position.y), items).into_any_element()
            }
        })
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.history.current();
        let title = current.file_name().map_or_else(
            || current.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let (back, forward, up) = (self.history.can_go_back(), self.history.can_go_forward(), current.parent().is_some());
        let active = self.active_view();
        let sort = format!("並べ替え: {} ▾", active.order.key.label());
        let group = format!("グループ: {} ▾", active.group.label());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(theme::TOOLBAR_HEIGHT))
            .px_3()
            .bg(rgb(theme::BAR))
            .border_b_1()
            .border_color(rgb(theme::LINE))
            .child(Self::tool_button("back", "‹", back).when(back, |b| b.on_click(cx.listener(|this, _, _, cx| this.back(cx)))))
            .child(Self::tool_button("forward", "›", forward).when(forward, |b| b.on_click(cx.listener(|this, _, _, cx| this.forward(cx)))))
            .child(Self::tool_button("parent", "↑", up).when(up, |b| b.on_click(cx.listener(|this, _, _, cx| this.parent(cx)))))
            .child(div().flex_1().min_w_0().px_2().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
            .child(self.menu_button("sort", sort, Menu::Sort, cx))
            .child(self.menu_button("group", group, Menu::Group, cx))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_1()
                    .p_0p5()
                    .rounded_md()
                    .bg(rgb(theme::SEGMENT))
                    .child(self.segment("mode-columns", "カラム", ViewMode::Columns, cx))
                    .child(self.segment("mode-list", "リスト", ViewMode::List, cx))
                    .child(self.segment("mode-icons", "アイコン", ViewMode::Icons, cx)),
            )
    }

    fn sidebar_item(&self, id: String, label: String, path: PathBuf, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.history.current() == path;
        div()
            .id(gpui::ElementId::Name(id.into()))
            .px_2()
            .py_0p5()
            .rounded_md()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .cursor(CursorStyle::PointingHand)
            .when(active, |b| b.bg(rgb(theme::HOVER)))
            .hover(|s| s.bg(rgb(theme::HOVER)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.navigate(path.clone(), cx)))
    }

    fn sidebar_heading(label: &'static str) -> impl IntoElement {
        div()
            .px_2()
            .pt_3()
            .pb_1()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .text_color(rgb(theme::SUB_TEXT))
            .child(label)
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let favorites: Vec<_> = self
            .favorites
            .iter()
            .map(|(name, path)| self.sidebar_item(format!("fav-{name}"), name.clone(), path.clone(), cx))
            .collect();
        let drives: Vec<_> = self
            .drives
            .iter()
            .map(|drive| {
                let label = drive.display().to_string().trim_end_matches('\\').to_string();
                self.sidebar_item(format!("drive-{label}"), label, drive.clone(), cx)
            })
            .collect();
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(theme::SIDEBAR_WIDTH))
            .p_2()
            .bg(rgb(theme::SIDEBAR))
            .border_r_1()
            .border_color(rgb(theme::LINE))
            .overflow_y_scroll()
            .child(Self::sidebar_heading("よく使う項目"))
            .children(favorites)
            .child(Self::sidebar_heading("場所"))
            .children(drives)
    }

    fn status(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pane = self.pane.read(cx);
        let state = &pane.state;
        let count = state.entries.len();
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap_4()
            .h(px(theme::STATUS_HEIGHT))
            .px_3()
            .bg(rgb(theme::BAR))
            .border_t_1()
            .border_color(rgb(theme::LINE))
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .text_color(rgb(theme::SUB_TEXT))
            .children(self.message.clone().map(|message| div().flex_none().text_color(rgb(theme::ERROR_TEXT)).child(message)))
            .child(div().flex_none().child(if state.loading {
                format!("読み込み中… {count} 項目")
            } else if pane.properties_loading() {
                format!("{count} 項目　プロパティを読み込み中… {}/{count}", pane.properties.len())
            } else {
                match self.selection.len() {
                    0 => format!("{count} 項目"),
                    selected => format!("{selected} / {count} 項目を選択"),
                }
            }))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(state.path.display().to_string()))
    }
}

fn window_handle(window: &Window) -> isize {
    match HasWindowHandle::window_handle(window).map(|handle| handle.as_raw()) {
        Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
        _ => 0,
    }
}

fn saved_view(store: &Option<Store>, path: &Path) -> Option<FolderView> {
    store.as_ref().and_then(|store| store.folder_view(path))
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window_active = window.is_window_active();
        self.sync_with_listing(window, cx);
        let error = self.pane.read(cx).state.error.clone();
        let body = match (error, self.view.mode) {
            (Some(message), _) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::SUB_TEXT))
                .child(message)
                .into_any_element(),
            (None, ViewMode::Columns) => self.render_columns(cx).into_any_element(),
            (None, ViewMode::List) => self.render_list(cx).into_any_element(),
            (None, ViewMode::Icons) => self.render_icons(window, cx).into_any_element(),
        };
        let toolbar = self.toolbar(cx);
        let sidebar = self.sidebar(cx);
        let status = self.status(cx);
        let menu = self.open_menu(cx);
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .text_size(px(theme::TEXT_SIZE))
            .font_family(theme::FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                let target = this.current_folder(cx);
                this.drop_paths(paths.paths(), target, window.modifiers().control, cx);
            }))
            .on_mouse_move(cx.listener(Self::drag))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.finish_drag(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.finish_drag(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(toolbar)
            .child(div().flex().flex_1().min_h_0().child(sidebar).child(div().flex().flex_col().flex_1().min_w_0().child(body)))
            .child(status)
            .children(menu)
    }
}
