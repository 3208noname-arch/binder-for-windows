//! 検索欄と検索結果。結果は1つのフォルダの一覧として扱い、リスト表示の仕組みをそのまま使う。

use crate::browser::{Browser, Column};
use crate::folder_pane::FolderPane;
use crate::text_input::{InputEvent, TextInput};
use crate::theme;
use filer_core::{Field, FolderView, GroupKey, ListColumn, SearchQuery, ViewMode};
use gpui::{Context, CursorStyle, Entity, Focusable, Subscription, Window, div, prelude::*, px, rgb};
use std::collections::HashMap;
use std::path::PathBuf;

/// 開いている検索欄。
pub struct SearchBox {
    pub input: Entity<TextInput>,
    /// 配下すべてを探す。偽なら今のフォルダだけ
    pub recursive: bool,
    _subscription: Subscription,
}

/// 検索の間だけ退避している、元の表示。閉じるとそのまま戻す。
pub struct SearchRun {
    pane: Entity<FolderPane>,
    view: FolderView,
    columns: Vec<Column>,
    active_column: usize,
    column_anchor: Option<usize>,
    expanded: HashMap<PathBuf, Entity<FolderPane>>,
}

impl SearchRun {
    /// 検索を始める前の表示形式。
    pub fn mode(&self) -> ViewMode {
        self.view.mode
    }

    /// 検索の間も、隠し項目の切り替えなどを届ける先。
    pub fn panes(&self) -> impl Iterator<Item = &Entity<FolderPane>> {
        std::iter::once(&self.pane).chain(self.columns.iter().map(|column| &column.pane)).chain(self.expanded.values())
    }
}

impl Browser {
    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(search) = &self.tab.search_box {
            window.focus(&search.input.focus_handle(cx));
            return;
        }
        let input = cx.new(|cx| TextInput::new(String::new(), 0..0, None, cx).plain());
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::Changed => this.run_search(cx),
            InputEvent::Submit => window.focus(&this.focus),
            InputEvent::Cancel => this.drop_search(cx),
        });
        window.focus(&input.focus_handle(cx));
        self.tab.search_box = Some(SearchBox { input, recursive: true, _subscription: subscription });
        cx.notify();
    }

    /// 検索欄を閉じて、元の表示へ戻す。
    pub fn drop_search(&mut self, cx: &mut Context<Self>) {
        if self.tab.search_box.take().is_some() {
            filer_shell::remove_ime_caret();
            self.refocus = true;
        }
        self.stop_search(cx);
        cx.notify();
    }

    /// 検索の結果を片付けて、退避していた表示を戻す。検索欄は開いたまま。
    fn stop_search(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self.tab.search_run.take() else { return };
        self.tab.pane = run.pane;
        self.tab.view = run.view;
        self.tab.columns = run.columns;
        self.tab.active_column = run.active_column;
        self.tab.column_anchor = run.column_anchor;
        self.tab.expanded = run.expanded;
        self.forget_rows();
        cx.notify();
    }

    fn forget_rows(&mut self) {
        self.tab.selection.clear();
        self.tab.tree_cache = None;
        self.tab.grid_data = None;
        self.tab.grid_list.reset(0);
    }

    /// 検索欄の文字で探す。空なら元の表示に戻す。
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = &self.tab.search_box else { return };
        let query = SearchQuery::new(&search.input.read(cx).text(), search.recursive);
        if query.terms.is_empty() {
            self.stop_search(cx);
        } else if self.tab.search_run.is_some() {
            self.tab.selection.clear();
            self.tab.pane.update(cx, |pane, cx| pane.set_query(query, cx));
        } else {
            self.start_search(query, cx);
        }
        cx.notify();
    }

    fn start_search(&mut self, query: SearchQuery, cx: &mut Context<Self>) {
        let root = self.current_folder(cx);
        let (order, show_hidden) = (self.tab.view.order, self.settings.show_hidden);
        let pane = cx.new(|cx| FolderPane::new_search(root, order, show_hidden, query, cx));
        cx.observe(&pane, |_, _, cx| cx.notify()).detach();
        let mut view = self.tab.view.clone();
        view.mode = ViewMode::List;
        view.group = GroupKey::None;
        if let Some(name) = view.columns.iter().position(|column| column.field == Field::Name) {
            let field = Field::Location;
            view.columns.insert(name + 1, ListColumn { field, width: field.default_width() });
        }
        let run = SearchRun {
            pane: std::mem::replace(&mut self.tab.pane, pane.clone()),
            view: std::mem::replace(&mut self.tab.view, view.clone()),
            columns: std::mem::replace(&mut self.tab.columns, vec![Column::new(pane, view)]),
            active_column: std::mem::take(&mut self.tab.active_column),
            column_anchor: self.tab.column_anchor.take(),
            expanded: std::mem::take(&mut self.tab.expanded),
        };
        self.tab.search_run = Some(run);
        self.forget_rows();
    }

    fn search_scope(&self, id: &'static str, label: &'static str, recursive: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.tab.search_box.as_ref().is_some_and(|search| search.recursive == recursive);
        div()
            .id(id)
            .px_2()
            .rounded(px(theme::SMALL_RADIUS))
            .cursor(CursorStyle::PointingHand)
            .hover(|style| style.bg(rgb(theme::HOVER)))
            .when(active, |chip| chip.bg(rgb(theme::SEGMENT)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(search) = &mut this.tab.search_box {
                    search.recursive = recursive;
                }
                this.run_search(cx);
            }))
    }

    /// 検索欄の下の、探す範囲を選ぶ帯。
    pub fn search_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        self.tab.search_box.as_ref()?;
        let folder = self.current_folder(cx);
        let name = folder.file_name().map_or_else(|| folder.display().to_string(), |name| name.to_string_lossy().into_owned());
        let state = &self.tab.pane.read(cx).state;
        let found = self.tab.search_run.is_some().then(|| {
            let count = state.entries.len();
            if state.loading { format!("{count} 件（検索中）") } else { format!("{count} 件") }
        });
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .h(px(theme::SEARCH_BAR_HEIGHT))
                .px_3()
                .bg(rgb(theme::BAR))
                .border_b_1()
                .border_color(rgb(theme::LINE))
                .text_size(px(theme::SMALL_TEXT_SIZE))
                .child(div().text_color(rgb(theme::SUB_TEXT)).child("検索:"))
                .child(self.search_scope("scope-all", "配下すべて", true, cx))
                .child(self.search_scope("scope-here", "このフォルダだけ", false, cx))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(rgb(theme::SUB_TEXT)).child(name))
                .children(found.map(|found| div().flex_none().ml_auto().text_color(rgb(theme::SUB_TEXT)).child(found))),
        )
    }
}
