use crate::actions::{Band, Renaming, RowPress, Spring, SpringTarget};
use crate::scrollbar::{BarFade, IconSizeDrag, ScrollDrag, ScrollTarget};
use crate::settings_view::{SettingsModel, SettingsView};
use crate::folder_pane::{FolderPane, tree_for};
use crate::icon_cache::IconCache;
use crate::icons_view::GridData;
use crate::list_tree::Flattened;
use crate::search::{SearchBox, SearchRun};
use crate::size_cache::SizeCache;
use crate::theme;
use filer_core::{
    AppSettings, Date, Entry, Field, FolderView, GroupKey, History, Selection, SortKey, SortOrder, ViewMode,
};
use filer_shell::{Operation, drive_label, drives, favorites, init_thread, preload_items_menu, today};
use filer_store::Store;
use gpui::{
    AnyElement, Bounds, Context, CursorStyle, DragMoveEvent, Entity, ExternalPaths, FocusHandle, Image, ListAlignment, ListState,
    MouseButton, MouseMoveEvent, Pixels, Point, ScrollHandle, Size, Task, TitlebarOptions, UniformListScrollHandle, Window,
    WindowBounds, WindowHandle, WindowOptions, div, img, prelude::*, px, rgb, size,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::collections::{HashMap, HashSet};
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
    pub bar: BarFade,
}

impl Column {
    pub fn new(pane: Entity<FolderPane>, view: FolderView) -> Self {
        Self { pane, scroll: UniformListScrollHandle::new(), selected: None, view, tree: None, bar: BarFade::default() }
    }
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
    pub search_box: Option<SearchBox>,
    pub search_run: Option<SearchRun>,
    /// 次の描画で、一覧へフォーカスを戻す
    pub refocus: bool,
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
    /// ダブルクリックで開き、左端に寄せている列
    pub column_anchor: Option<usize>,
    /// 次の描画で、右端の列が見えるよう横へ送る
    pub columns_reveal: bool,
    pub resizing: Option<Resize>,
    pub header_drag: Option<HeaderDrag>,
    pub settings: AppSettings,
    pub settings_model: Entity<SettingsModel>,
    settings_window: Option<WindowHandle<SettingsView>>,
    pub suppress_click: bool,
    /// 今の押下が `grab_zone` の上だった
    pub grab_press: bool,
    pub band_task: Option<Task<()>>,
    pub scale: f32,
    pub viewport: Size<Pixels>,
    pub bars: HashMap<ScrollTarget, BarFade>,
    bars_wake: Option<(Instant, Task<()>)>,
    drag_seen: bool,
    pub cut: HashSet<PathBuf>,
    pub spring: Option<Spring>,
    pub springs_done: u32,
    pub band: Option<Band>,
    pub scroll_drag: Option<ScrollDrag>,
    pub icon_size_drag: Option<IconSizeDrag>,
    pub sidebar_scroll: ScrollHandle,
    pub hovered_cell: Option<PathBuf>,
    pub rename_request: Option<PathBuf>,
    pub drive_labels: HashMap<PathBuf, String>,
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
        let settings = store.as_ref().map(Store::app_settings).unwrap_or_default();
        let settings_model = cx.new(|_| SettingsModel { value: settings.clone() });
        cx.observe(&settings_model, |this, model, cx| {
            let settings = model.read(cx).value.clone();
            let show_hidden = settings.show_hidden;
            let changed = show_hidden != this.settings.show_hidden;
            this.settings = settings;
            if changed {
                this.apply_show_hidden(show_hidden, cx);
            }
            cx.notify();
        })
        .detach();
        let mut view = FolderView::for_folder(saved_view(&store, &path), ViewMode::List);
        if let Some(mode) = mode {
            view.mode = mode;
        }
        let (order, wants, show_hidden) = (view.order, view.needs_properties(), settings.show_hidden);
        let pane = cx.new(|cx| {
            let mut pane = FolderPane::new(path.clone(), order, show_hidden, cx);
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
            search_box: None,
            search_run: None,
            refocus: false,
            pending_rename: None,
            press: None,
            undo: Vec::new(),
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
            resizing: None,
            header_drag: None,
            settings,
            settings_model,
            settings_window: None,
            suppress_click: false,
            grab_press: false,
            band_task: None,
            scale: window.scale_factor(),
            viewport: window.viewport_size(),
            bars: HashMap::new(),
            bars_wake: None,
            drag_seen: false,
            cut: HashSet::new(),
            spring: None,
            springs_done: 0,
            band: None,
            scroll_drag: None,
            icon_size_drag: None,
            sidebar_scroll: ScrollHandle::new(),
            hovered_cell: None,
            rename_request: None,
            drive_labels: HashMap::new(),
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
        browser.reset_columns(cx);
        browser.load_in_background(cx);
        browser
    }

    /// ドライブ名と右クリックメニューの準備を裏で行う。切断されたドライブがあると待たされるため。
    fn load_in_background(&self, cx: &mut Context<Self>) {
        let drives = self.drives.clone();
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            init_thread();
            let labels: HashMap<PathBuf, String> =
                drives.into_iter().filter_map(|drive| Some((drive.clone(), drive_label(&drive)?))).collect();
            let _ = tx.send(labels);
            // 初回の右クリックの遅れは、シェル拡張の読み込みに使われている
            let _ = preload_items_menu(Path::new(r"C:\Windows"), &["explorer.exe".into()]);
        });
        cx.spawn(async move |this, cx| {
            if let Ok(labels) = rx.await {
                let _ = this.update(cx, |browser, cx| {
                    browser.drive_labels = labels;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 設定のウィンドウを開く。開いていれば前に出す。
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(window) = self.settings_window
            && window.update(cx, |_, window, _| window.activate_window()).is_ok()
        {
            return;
        }
        let model = self.settings_model.clone();
        let bounds = Bounds::centered(None, size(px(theme::SETTINGS_WINDOW_WIDTH), px(theme::SETTINGS_WINDOW_HEIGHT)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("設定".into()), ..Default::default() }),
            ..Default::default()
        };
        self.settings_window = cx.open_window(options, move |_, cx| cx.new(|cx| SettingsView::new(model, cx))).ok();
    }

    /// 隠し項目の表示を切り替える。設定の窓と同じ値を保存する。
    pub fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.settings_model.update(cx, |model, cx| {
            model.value.show_hidden = !model.value.show_hidden;
            cx.notify();
        });
        if let Some(store) = &self.store
            && let Err(error) = store.save_app_settings(&self.settings_model.read(cx).value)
        {
            eprintln!("設定を保存できませんでした: {error}");
        }
    }

    fn apply_show_hidden(&mut self, show: bool, cx: &mut Context<Self>) {
        let saved = self.search_run.iter().flat_map(|run| run.panes()).cloned();
        let panes = std::iter::once(self.pane.clone())
            .chain(self.columns.iter().map(|column| column.pane.clone()))
            .chain(self.expanded.values().cloned())
            .chain(saved);
        for pane in panes.collect::<Vec<_>>() {
            pane.update(cx, |pane, cx| pane.set_show_hidden(show, cx));
        }
    }

    /// スクロールされたものを調べ、スクロールバーのフェードが続く間は描き直す。
    fn watch_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        for target in ScrollTarget::FIXED {
            let offset = self.bar_offset(target);
            self.bars.entry(target).or_default().observe(offset, now);
        }
        for index in 0..self.columns.len() {
            let offset = self.bar_offset(ScrollTarget::Column(index));
            self.columns[index].bar.observe(offset, now);
        }
        let fades = || self.bars.values().chain(self.columns.iter().map(|column| &column.bar));
        if fades().any(|bar| bar.fading(now)) {
            window.request_animation_frame();
            return;
        }
        let Some(wake) = fades().filter_map(|bar| bar.fade_start(now)).min() else { return };
        if self.bars_wake.as_ref().is_some_and(|(at, _)| *at == wake) {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wake.saturating_duration_since(Instant::now())).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        });
        self.bars_wake = Some((wake, task));
    }

    pub fn new_pane(&self, path: PathBuf, order: SortOrder, wants: bool, cx: &mut Context<Self>) -> Entity<FolderPane> {
        let show_hidden = self.settings.show_hidden;
        let pane = cx.new(|cx| {
            let mut pane = FolderPane::new(path, order, show_hidden, cx);
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

    /// 列を作り直す。カラム表示では親の列も並べ、開いたフォルダを左端に寄せる。
    fn reset_columns(&mut self, cx: &mut Context<Self>) {
        let mut previous = std::mem::take(&mut self.columns);
        let mut columns = Vec::new();
        if self.view.mode == ViewMode::Columns {
            let path = self.history.current().to_path_buf();
            let mut chain: Vec<&Path> = path.ancestors().collect();
            chain.reverse();
            for pair in chain.windows(2) {
                let (folder, child) = (pair[0], pair[1]);
                let reused = previous.iter().position(|column| column.pane != self.pane && column.pane.read(cx).state.path == folder);
                let mut column = match reused {
                    Some(index) => previous.remove(index),
                    None => {
                        let view = self.view_of(folder);
                        Column::new(self.new_pane(folder.to_path_buf(), view.order, view.needs_properties(), cx), view)
                    }
                };
                column.selected = child.file_name().map(|name| name.to_os_string());
                columns.push(column);
            }
        }
        columns.push(Column::new(self.pane.clone(), self.view.clone()));
        self.active_column = columns.len() - 1;
        self.column_anchor = (self.active_column > 0).then_some(self.active_column);
        self.columns_reveal = true;
        self.columns = columns;
        self.preview = None;
    }

    /// 履歴を増やして移動する。
    pub fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.history.visit(path.clone());
        self.show(path, cx);
    }

    fn show(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.drop_search(cx);
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
        self.reset_columns(cx);
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        self.drop_search(cx);
        if self.leave_pinned_column(cx) {
            return;
        }
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
        self.drop_search(cx);
        if self.leave_pinned_column(cx) {
            return;
        }
        if let Some(parent) = self.current_folder(cx).parent().map(PathBuf::from) {
            self.navigate(parent, cx);
        }
    }

    /// 操作中の列が、履歴の現在地より深い列か。
    fn in_child_column(&self) -> bool {
        self.view.mode == ViewMode::Columns && self.columns[self.active_column].pane != self.pane
    }

    /// 開いているフォルダの設定を変え、すぐ保存する。カラム表示では操作中の列が対象。
    pub fn update_view(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut FolderView)) {
        let in_child_column = self.in_child_column();
        let (order, wants, pane) = if in_child_column {
            let column = &mut self.columns[self.active_column];
            change(&mut column.view);
            (column.view.order, column.view.needs_properties(), column.pane.clone())
        } else {
            change(&mut self.view);
            if let Some(column) = self.columns.iter_mut().find(|column| column.pane == self.pane) {
                column.view = self.view.clone();
            }
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
        // 検索の結果の表示は、そのフォルダの設定ではない
        let Some(store) = self.store.as_ref().filter(|_| self.search_run.is_none()) else { return };
        let result = if self.in_child_column() {
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

    /// 表示形式を変える。見ていたフォルダはそのまま引き継ぐ。
    fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.drop_search(cx);
        if mode == self.view.mode {
            return;
        }
        let location = self.current_folder(cx);
        if location != self.history.current() {
            self.navigate(location, cx);
        }
        self.update_view(cx, |view| view.mode = mode);
        self.reset_columns(cx);
    }

    /// 並べ替えとグループの対象になっているフォルダの設定。
    fn active_view(&self) -> &FolderView {
        if self.in_child_column() { &self.columns[self.active_column].view } else { &self.view }
    }

    /// 選択の色。別のウィンドウへ移った時は灰色にする。
    pub fn selection_colors(&self) -> (u32, u32) {
        if self.selection.is_soft() {
            (theme::SELECTED_SOFT, theme::TEXT)
        } else if self.window_active {
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
            self.scroll_drag = None;
            if self.resizing.take().is_some() || self.icon_size_drag.take().is_some() {
                self.save_active_view(cx);
            }
            self.header_drag = None;
            return;
        }
        if self.press.is_some() {
            self.drag_row(event, cx);
        } else if self.scroll_drag.is_some() {
            self.drag_scroll(event, cx);
        } else if self.icon_size_drag.is_some() {
            self.drag_icon_size(event, cx);
        } else if self.resizing.is_some() {
            self.drag_resize(event, cx);
        } else if self.header_drag.is_some() {
            self.drag_header(event, window, cx);
        }
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        self.end_press();
        self.end_spring();
        self.scroll_drag = None;
        if self.drag_seen {
            self.drag_seen = false;
            cx.notify();
        }
        if self.resizing.take().is_some() || self.icon_size_drag.take().is_some() {
            self.save_active_view(cx);
        }
        if let Some(drag) = self.header_drag.take() {
            self.drop_header(drag, cx);
        }
    }

    /// 項目のアイコン。ショートカットには矢印の印を重ねる。
    pub fn item_icon(&self, entry: &Entry, icon: Option<Arc<Image>>, size: f32) -> AnyElement {
        let base = self.icon(icon, entry.is_dir, size);
        if !entry.is_shortcut() {
            return base;
        }
        let badge = (size * theme::SHORTCUT_BADGE_RATIO).clamp(theme::SHORTCUT_BADGE_MIN, theme::SHORTCUT_BADGE_MAX);
        div()
            .relative()
            .flex_none()
            .size(px(size))
            .child(base)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(badge))
                    .rounded_sm()
                    .bg(rgb(theme::BACKGROUND))
                    .border_1()
                    .border_color(rgb(theme::LINE))
                    .text_size(px(badge * 0.8))
                    .line_height(px(badge))
                    .text_color(rgb(theme::SHORTCUT_ARROW))
                    .child("↗"),
            )
            .into_any_element()
    }

    /// 薄く出す項目か。切り取りの途中と、隠し項目。
    pub fn is_dimmed(&self, path: &Path, entry: &Entry) -> bool {
        self.is_cut(path) || entry.is_hidden
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

    /// 戻る、進む、上へのボタン。よく使うので、狙いやすい大きさにする。
    fn nav_button(id: &'static str, label: &'static str, enabled: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(theme::NAV_BUTTON_WIDTH))
            .h(px(theme::NAV_BUTTON_HEIGHT))
            .rounded_full()
            .text_size(px(theme::NAV_GLYPH_SIZE))
            .line_height(px(theme::NAV_BUTTON_HEIGHT))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(if enabled { theme::TEXT } else { theme::DISABLED_TEXT }))
            .when(enabled, |b| b.cursor(CursorStyle::PointingHand).hover(|s| s.bg(rgb(theme::HOVER))))
            .child(label)
    }

    /// Finderと同じ、白い丸い枠にまとめたボタンの組。
    fn nav_group() -> gpui::Div {
        div().flex().flex_none().items_center().gap_0p5().p_0p5().rounded_full().bg(rgb(theme::BACKGROUND)).border_1().border_color(rgb(theme::LINE))
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
        let shift = if self.search_box.is_some() { theme::SEARCH_FIELD_WIDTH - theme::SEARCH_BUTTON_WIDTH } else { 0. };
        Some(match self.menu? {
            Menu::Sort => {
                let items = self.sort_menu(cx);
                Self::popup(|d| d.top(top).right(px(theme::MENU_RIGHT_SORT + shift)), items).into_any_element()
            }
            Menu::Group => {
                let items = self.group_menu(cx);
                Self::popup(|d| d.top(top).right(px(theme::GROUP_MENU_RIGHT + shift)), items).into_any_element()
            }
            Menu::Fields(position) => {
                let items = self.fields_menu(cx);
                Self::popup(|d| d.left(position.x).top(position.y), items).into_any_element()
            }
        })
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.current_folder(cx);
        let title = current.file_name().map_or_else(
            || current.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let pinned = self.column_pinned();
        let (back, forward, up) = (self.history.can_go_back() || pinned, self.history.can_go_forward(), current.parent().is_some() || pinned);
        let parent_folder = current.parent().map(PathBuf::from);
        let active = self.active_view();
        let sort = format!("並べ替え: {} ▾", active.order.key.label());
        let group = format!("グループ: {} ▾", active.group.label());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(theme::TOOLBAR_HEIGHT))
            .px_3()
            .bg(rgb(theme::BAR))
            .border_b_1()
            .border_color(rgb(theme::LINE))
            .child(
                Self::nav_group()
                    .child(Self::nav_button("back", "‹", back).when(back, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.back(cx))).on_drag_move::<ExternalPaths>(cx.listener(
                            |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                                this.hover_target(SpringTarget::Back, event.bounds, event.event.position, cx);
                            },
                        ))
                    }))
                    .child(div().flex_none().w(px(1.)).h(px(theme::NAV_DIVIDER_HEIGHT)).bg(rgb(theme::LINE)))
                    .child(Self::nav_button("forward", "›", forward).when(forward, |b| b.on_click(cx.listener(|this, _, _, cx| this.forward(cx))))),
            )
            .child(Self::nav_group().child(Self::nav_button("parent", "↑", up).when(up, |b| {
                let target = parent_folder.clone();
                b.on_click(cx.listener(|this, _, _, cx| this.parent(cx)))
                    .on_drag_move::<ExternalPaths>(cx.listener(|this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                        this.hover_target(SpringTarget::Parent, event.bounds, event.event.position, cx);
                    }))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        cx.stop_propagation();
                        let copy = window.modifiers().control;
                        if let Some(target) = target.clone() {
                            this.drop_paths(paths.paths(), target, copy, cx);
                        }
                    }))
            })))
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
            .child(
                Self::tool_button("hidden", "隠し項目", true)
                    .when(self.settings.show_hidden, |button| button.bg(rgb(theme::SEGMENT)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_hidden(cx))),
            )
            .child(match &self.search_box {
                Some(search) => div().flex_none().w(px(theme::SEARCH_FIELD_WIDTH)).child(search.input.clone()).into_any_element(),
                None => Self::tool_button("search", "検索", true)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| this.open_search(window, cx)))
                    .into_any_element(),
            })
            .child(
                Self::tool_button("settings", "設定", true)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
            )
    }

    fn sidebar_item(&self, id: String, label: String, path: PathBuf, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.history.current() == path;
        let (hover_path, drop_path) = (path.clone(), path.clone());
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
            .drag_over::<ExternalPaths>(|style, _, _, _| style.bg(rgb(theme::HOVER)))
            .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                this.hover_target(SpringTarget::Go(hover_path.clone()), event.bounds, event.event.position, cx);
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                cx.stop_propagation();
                let copy = window.modifiers().control;
                this.drop_paths(paths.paths(), drop_path.clone(), copy, cx);
            }))
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
                let short = drive.display().to_string().trim_end_matches('\\').to_string();
                let label = self.drive_labels.get(drive).cloned().unwrap_or_else(|| short.clone());
                self.sidebar_item(format!("drive-{short}"), label, drive.clone(), cx)
            })
            .collect();
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .h_full()
            .w(px(theme::SIDEBAR_WIDTH))
            .p_2()
            .bg(rgb(theme::SIDEBAR))
            .border_r_1()
            .border_color(rgb(theme::LINE))
            .overflow_y_scroll()
            .track_scroll(&self.sidebar_scroll)
            .child(Self::sidebar_heading("よく使う項目"))
            .children(favorites)
            .child(Self::sidebar_heading("場所"))
            .children(drives)
    }

    fn status(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let icon_size = (self.view.mode == ViewMode::Icons).then(|| self.icon_size_control(cx));
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
            .children(icon_size)
            .children(self.message.clone().map(|message| div().flex_none().text_color(rgb(theme::ERROR_TEXT)).child(message)))
            .child(div().flex_none().child(if state.loading {
                let doing = if self.search_run.is_some() { "検索中" } else { "読み込み中" };
                format!("{doing}… {count} 項目")
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

/// ドラッグの移動かコピーかの判断に使う、パスのドライブ部分。
fn volume_of(path: &Path) -> String {
    path.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).unwrap_or_default()
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
        self.scale = window.scale_factor();
        self.viewport = window.viewport_size();
        if std::mem::take(&mut self.refocus) {
            window.focus(&self.focus);
        }
        self.watch_scroll(window, cx);
        self.sync_with_listing(window, cx);
        if let Some(path) = self.rename_request.take() {
            self.begin_rename(path, window, cx);
        }
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
        let search_bar = self.search_bar(cx);
        let sidebar = self.sidebar(cx);
        let status = self.status(cx);
        let menu = self.open_menu(cx);
        let band = self.band_view();
        let dragging = cx.has_active_drag();
        if !dragging {
            self.end_spring();
        }
        let drag_bar = dragging.then(|| self.drag_bar(cx));
        let sidebar_bar = self.scrollbar("sidebar-bar", ScrollTarget::Sidebar, true, 0., cx);
        gpui::set_drop_volume(&volume_of(self.history.current()));
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
            .children(search_bar)
            .on_drag_move::<ExternalPaths>(cx.listener(|this, _: &DragMoveEvent<ExternalPaths>, _, cx| {
                if !this.drag_seen {
                    this.drag_seen = true;
                    cx.notify();
                }
            }))
            .child(div().flex().flex_1().min_h_0().child(div().relative().flex_none().h_full().child(sidebar).children(sidebar_bar)).child(div().flex().flex_col().flex_1().min_w_0().child(body)))
            .child(status)
            .children(band)
            .children(drag_bar)
            .children(menu)
    }
}
