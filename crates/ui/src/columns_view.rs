use crate::browser::{Browser, Column, Resize};
use crate::icon_cache::{LARGE, SMALL};
use crate::list_tree::{Flattened, Row};
use crate::scrollbar::ScrollTarget;
use crate::theme;
use filer_core::{Entry, fit_width, format_size, kind_of};
use filer_shell::format_time;
use gpui::{
    ClickEvent, Context, CursorStyle, ExternalPaths, MouseButton, MouseDownEvent, Point, UniformListScrollHandle, Window, div,
    prelude::*, px, rgb, uniform_list,
};
use std::ffi::OsString;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

/// 列の幅を範囲に収める。
fn clamp_column_width(width: f32) -> f32 {
    width.clamp(theme::COLUMN_MIN_WIDTH, theme::COLUMN_MAX_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::clamp_column_width;
    use crate::theme::{COLUMN_MAX_WIDTH, COLUMN_MIN_WIDTH};

    #[test]
    fn width_stays_within_limits() {
        assert_eq!(clamp_column_width(280.), 280.);
        assert_eq!(clamp_column_width(0.), COLUMN_MIN_WIDTH);
        assert_eq!(clamp_column_width(5000.), COLUMN_MAX_WIDTH);
    }
}

impl Browser {
    pub fn resize_column_view(&mut self, column: usize, width: f32, cx: &mut Context<Self>) {
        let width = clamp_column_width(width);
        if let Some(current) = self.columns.get_mut(column) {
            current.view.column_width = width;
            if column == 0 {
                self.view.column_width = width;
            }
            self.active_column = column;
            cx.notify();
        }
    }

    /// 列の境界のダブルクリック。いちばん長い名前に合わせる。
    fn fit_column(&mut self, column: usize, cx: &mut Context<Self>) {
        let Some(current) = self.columns.get(column) else { return };
        let state = &current.pane.read(cx).state;
        let names = state.entries.iter().map(|entry| entry.name.as_str());
        let width = fit_width(names, theme::TEXT_SIZE, theme::COLUMN_FIT_PADDING);
        self.resize_column_view(column, width, cx);
        self.save_active_view(cx);
    }

    /// 列の余白を押した時は、その列の選択を解いて右の列を閉じる。
    fn clear_column(&mut self, column: usize, cx: &mut Context<Self>) {
        if let Some(current) = self.columns.get_mut(column) {
            current.selected = None;
            self.selection.clear();
            self.columns.truncate(column + 1);
            self.active_column = column;
            self.preview = None;
            cx.notify();
        }
    }

    /// 列のクリック。Ctrl、Shiftは選択だけを変え、普通のクリックは列を開く。
    #[allow(clippy::too_many_arguments)]
    fn click_column_item(&mut self, column: usize, path: PathBuf, name: OsString, entry: Entry, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.active_column = column;
        let modifiers = event.modifiers();
        if event.click_count() == 2 || modifiers.control || modifiers.shift {
            let is_dir = entry.is_dir;
            self.click_item(path, is_dir, event, window, cx);
        } else {
            self.select_in_column(column, name, entry, cx);
            window.focus(&self.focus);
        }
    }

    /// 列の項目を選ぶ。右の列を閉じ、フォルダなら新しい列を開く。
    pub fn select_in_column(&mut self, column: usize, name: OsString, entry: Entry, cx: &mut Context<Self>) {
        let Some(current) = self.columns.get_mut(column) else { return };
        current.selected = Some(name.clone());
        self.active_column = column;
        let folder = current.pane.read(cx).state.path.clone();
        let path = folder.join(&name);
        self.selection.select_only(path.clone());
        self.columns.truncate(column + 1);
        if entry.is_dir {
            self.preview = None;
            let view = self.view_of(&path);
            let pane = self.new_pane(path, view.order, view.needs_properties(), cx);
            self.columns.push(Column { pane, scroll: UniformListScrollHandle::new(), selected: None, view, tree: None });
            self.active_column = column + 1;
        } else {
            self.preview = Some((folder, entry));
        }
        // 右端の新しい列が見えるよう、最後まで送る。範囲は描画の時に収められる
        self.columns_scroll.set_offset(Point::new(px(-1_000_000.), px(0.)));
        cx.notify();
    }

    pub fn column_tree(&mut self, column: usize, cx: &Context<Self>) -> Rc<Flattened> {
        let current = &self.columns[column];
        let group = current.view.group;
        let pane = current.pane.read(cx);
        if let Some((key, tree)) = &current.tree
            && key.root_version == pane.version
            && key.group == group
            && key.today == filer_shell::today()
        {
            return tree.clone();
        }
        let (key, tree) = Self::plain_tree(pane, group);
        let tree = Rc::new(tree);
        self.columns[column].tree = Some((key, tree.clone()));
        tree
    }

    fn column_rows(&mut self, column: usize, tree: &Flattened, range: Range<usize>, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let Some(current) = self.columns.get(column) else { return Vec::new() };
        let selected = current.selected.clone();
        let pane = current.pane.clone();
        let (selected_background, selected_text) = self.selection_colors();
        let folder: PathBuf = pane.read(cx).state.path.clone();
        range
            .filter_map(|ix| {
                let row = match tree.rows.get(ix)? {
                    Row::Header { label, count } => {
                        let ordinal = tree.rows[..ix].iter().filter(|row| matches!(row, Row::Header { .. })).count();
                        return Some(self.group_header(ix, ordinal, label, *count, Some(column), cx));
                    }
                    Row::Item(row) => *row,
                };
                let entry = pane.read(cx).state.entries.get(row.entry as usize)?.clone();
                let path = folder.join(entry.os_name());
                let active = selected.as_deref() == Some(entry.os_name()) || self.selection.contains(&path);
                let background = match (active, ix % 2 == 1) {
                    (true, _) => selected_background,
                    (false, true) => theme::STRIPE,
                    (false, false) => theme::BACKGROUND,
                };
                let icon = self.icons.update(cx, |icons, _| icons.get(&folder, &entry, SMALL));
                let name = entry.os_name().to_os_string();
                let is_dir = entry.is_dir;
                let icon = self.icon(icon, is_dir, theme::ROW_ICON);
                let name_element = self.name_or_input(&path, &entry);
                let is_cut = self.is_cut(&path);
                let row = div()
                    .id(ix)
                    .when(is_cut, |row| row.opacity(theme::CUT_OPACITY))
                        .flex()
                        .items_center()
                        .gap_1()
                        .w_full()
                        .h(px(theme::ROW_HEIGHT))
                        .px_2()
                        .bg(rgb(background))
                        .text_color(rgb(if active { selected_text } else { theme::TEXT }))
                        .when(!active, |row| row.hover(|s| s.bg(rgb(theme::HOVER))))
                        .child(icon)
                    .child(div().flex_1().min_w_0().overflow_hidden().child(name_element))
                    .when(is_dir, |row| row.child(div().flex_none().text_size(px(theme::SMALL_TEXT_SIZE)).child("›")));
                let click_path = path.clone();
                Some(
                    self.item_mouse(row, path, is_dir, cx)
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            this.click_column_item(column, click_path.clone(), name.clone(), entry.clone(), event, window, cx)
                        }))
                        .into_any_element(),
                )
            })
            .collect()
    }

    fn column_list(&mut self, column: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tree = self.column_tree(column, cx);
        let current = &self.columns[column];
        let error = current.pane.read(cx).state.error.clone();
        let scroll = current.scroll.clone();
        let width = current.view.column_width;
        let body = match error {
            Some(message) => div().p_2().text_color(rgb(theme::SUB_TEXT)).child(message).into_any_element(),
            None => {
                let count = tree.rows.len();
                uniform_list(
                    ("column", column),
                    count,
                    cx.processor(move |this, range, _: &mut Window, cx| this.column_rows(column, &tree, range, cx)),
                )
                .track_scroll(scroll)
                .size_full()
                .into_any_element()
            }
        };
        let bar = self.scrollbar(Self::column_bar_id(column), ScrollTarget::Column(column), true, 0., cx);
        let handle = div()
            .id(("column-handle", column))
            .absolute()
            .top_0()
            .right_0()
            .h_full()
            .w(px(theme::COLUMN_HANDLE_WIDTH))
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(|s| s.bg(rgb(theme::LINE)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.active_column = column;
                    if event.click_count == 2 {
                        this.fit_column(column, cx);
                    } else {
                        this.resizing = Some(Resize::Column { column, start_x: event.position.x.into(), start_width: width });
                    }
                }),
            );
        div()
            .relative()
            .flex_none()
            .w(px(width))
            .h_full()
            .border_r_1()
            .border_color(rgb(theme::LINE))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                this.commit_rename(window, cx);
                this.clear_column(column, cx);
                this.begin_band(event.position, Some(column), false, cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| {
                this.active_column = column;
                this.folder_menu(cx);
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                cx.stop_propagation();
                let copy = window.modifiers().control;
                let target = this.columns[column].pane.read(cx).state.path.clone();
                this.drop_paths(paths.paths(), target, copy, cx);
            }))
            .child(body)
            .children(bar)
            .child(handle)
    }

    fn preview_panel(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (folder, entry) = self.preview.as_ref()?;
        let icon = self.icons.update(cx, |icons, _| icons.get(folder, entry, LARGE));
        let line = |label: &str, value: String| {
            div()
                .flex()
                .w_full()
                .gap_2()
                .text_size(px(theme::SMALL_TEXT_SIZE))
                .child(div().flex_none().text_color(rgb(theme::SUB_TEXT)).child(label.to_string()))
                .child(div().min_w_0().overflow_hidden().child(value))
        };
        Some(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .items_center()
                .gap_2()
                .p_4()
                .w(px(theme::PREVIEW_WIDTH))
                .h_full()
                .child(self.icon(icon, false, theme::PREVIEW_ICON))
                .child(div().w_full().text_center().overflow_hidden().child(entry.name.clone()))
                .child(line("種類", kind_of(entry)))
                .child(line("サイズ", format_size(entry.size)))
                .child(line("更新日時", format_time(entry.modified))),
        )
    }

    pub fn render_columns(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let columns: Vec<_> = (0..self.columns.len()).map(|i| self.column_list(i, cx)).collect();
        let preview = self.preview_panel(cx);
        let bar = self.scrollbar("columns-bar", ScrollTarget::Columns, false, 0., cx);
        let mut strip = div()
            .id("columns")
            .flex()
            .flex_1()
            .min_h_0()
            .overflow_x_scroll()
            .track_scroll(&self.columns_scroll)
            .children(columns)
            .children(preview);
        // 列の縦スクロールで、並び全体まで横に動かないようにする
        strip.style().restrict_scroll_to_axis = Some(true);
        div().relative().flex().flex_1().min_h_0().child(strip).children(bar)
    }

    /// 列ごとのスクロールバーの名前。列は多くならないので、固定の名前で足りる。
    fn column_bar_id(column: usize) -> &'static str {
        const IDS: [&str; 8] = ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"];
        IDS[column.min(IDS.len() - 1)]
    }
}
