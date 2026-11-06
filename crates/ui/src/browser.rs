use crate::folder_pane::FolderPane;
use crate::icon_cache::IconCache;
use crate::theme;
use filer_core::{Entry, History, SortKey, SortOrder};
use filer_shell::{drives, favorites};
use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Entity, Image, ScrollHandle,
    UniformListScrollHandle, Window, div, img, prelude::*, px, rgb,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Columns,
    List,
    Icons,
}

pub struct Column {
    pub pane: Entity<FolderPane>,
    pub scroll: UniformListScrollHandle,
    pub selected: Option<OsString>,
}

/// ウィンドウ全体。履歴と表示形式を持ち、3つの表示へ分ける。
pub struct Browser {
    pub history: History,
    pub mode: ViewMode,
    pub pane: Entity<FolderPane>,
    pub order: SortOrder,
    pub selected: Option<PathBuf>,
    pub list_scroll: UniformListScrollHandle,
    pub grid_scroll: UniformListScrollHandle,
    pub columns: Vec<Column>,
    pub columns_scroll: ScrollHandle,
    pub preview: Option<(PathBuf, Entry)>,
    pub expanded: HashMap<PathBuf, Entity<FolderPane>>,
    pub icons: Entity<IconCache>,
    favorites: Vec<(String, PathBuf)>,
    drives: Vec<PathBuf>,
}

impl Browser {
    pub fn new(path: PathBuf, measure: Option<Instant>, cx: &mut Context<Self>) -> Self {
        let order = SortOrder { key: SortKey::Name, descending: false };
        let pane = cx.new(|cx| {
            let mut pane = FolderPane::new(path.clone(), order, cx);
            if let Some(start) = measure {
                pane.measure_from(start);
            }
            pane
        });
        cx.observe(&pane, |_, _, cx| cx.notify()).detach();
        let icons = cx.new(IconCache::new);
        cx.observe(&icons, |_, _, cx| cx.notify()).detach();
        let mut browser = Self {
            history: History::new(path),
            mode: ViewMode::List,
            pane,
            order,
            selected: None,
            list_scroll: UniformListScrollHandle::new(),
            grid_scroll: UniformListScrollHandle::new(),
            columns: Vec::new(),
            columns_scroll: ScrollHandle::new(),
            preview: None,
            expanded: HashMap::new(),
            icons,
            favorites: favorites(),
            drives: drives(),
        };
        browser.reset_columns();
        browser
    }

    pub fn new_pane(&self, path: PathBuf, cx: &mut Context<Self>) -> Entity<FolderPane> {
        let order = self.order;
        let pane = cx.new(|cx| FolderPane::new(path, order, cx));
        cx.observe(&pane, |_, _, cx| cx.notify()).detach();
        pane
    }

    fn reset_columns(&mut self) {
        let pane = self.pane.clone();
        self.columns = vec![Column { pane, scroll: UniformListScrollHandle::new(), selected: None }];
        self.preview = None;
    }

    /// 履歴を増やして移動する。
    pub fn navigate(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.history.visit(path.clone());
        self.show(path, cx);
    }

    fn show(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.selected = None;
        self.expanded.clear();
        self.list_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.grid_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.pane.update(cx, |pane, cx| pane.open(path, cx));
        self.reset_columns();
        cx.notify();
    }

    fn go_back(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.history.go_back().map(PathBuf::from) {
            self.show(path, cx);
        }
    }

    fn go_forward(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.history.go_forward().map(PathBuf::from) {
            self.show(path, cx);
        }
    }

    fn open_parent(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = self.history.current().parent().map(PathBuf::from) {
            self.navigate(parent, cx);
        }
    }

    pub fn toggle_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        let descending = self.order.key == key && !self.order.descending;
        self.order = SortOrder { key, descending };
        let order = self.order;
        let panes = std::iter::once(self.pane.clone())
            .chain(self.expanded.values().cloned())
            .chain(self.columns.iter().map(|c| c.pane.clone()));
        for pane in panes.collect::<Vec<_>>() {
            pane.update(cx, |pane, cx| pane.set_order(order, cx));
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
        let active = self.mode == mode;
        div()
            .id(id)
            .px_2()
            .rounded_sm()
            .cursor(CursorStyle::PointingHand)
            .when(active, |b| b.bg(rgb(theme::BACKGROUND)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.mode = mode;
                cx.notify();
            }))
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.history.current();
        let title = current.file_name().map_or_else(
            || current.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let (back, forward, up) = (self.history.can_go_back(), self.history.can_go_forward(), current.parent().is_some());
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
            .child(Self::tool_button("back", "‹", back).when(back, |b| b.on_click(cx.listener(Self::go_back))))
            .child(Self::tool_button("forward", "›", forward).when(forward, |b| b.on_click(cx.listener(Self::go_forward))))
            .child(Self::tool_button("parent", "↑", up).when(up, |b| b.on_click(cx.listener(Self::open_parent))))
            .child(div().flex_1().min_w_0().px_2().overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
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
        let state = &self.pane.read(cx).state;
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
            .child(div().flex_none().child(if state.loading {
                format!("読み込み中… {count} 項目")
            } else {
                format!("{count} 項目")
            }))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(state.path.display().to_string()))
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = self.pane.read(cx).state.error.clone();
        let body = match (error, self.mode) {
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
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .text_size(px(theme::TEXT_SIZE))
            .font_family(theme::FONT)
            .child(toolbar)
            .child(div().flex().flex_1().min_h_0().child(sidebar).child(div().flex().flex_col().flex_1().min_w_0().child(body)))
            .child(status)
    }
}
