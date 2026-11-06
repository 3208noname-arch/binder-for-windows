use crate::browser::{Browser, Column, ColumnResize};
use crate::icon_cache::{LARGE, SMALL};
use crate::theme;
use filer_core::{Entry, format_size, kind_of};
use filer_shell::format_time;
use gpui::{
    ClickEvent, Context, CursorStyle, MouseButton, MouseDownEvent, MouseMoveEvent, Point, UniformListScrollHandle,
    Window, div, prelude::*, px, rgb, uniform_list,
};
use std::ffi::OsString;
use std::ops::Range;
use std::path::PathBuf;

/// ドラッグの量から列の幅を決める。
fn resized_width(start_width: f32, start_x: f32, x: f32) -> f32 {
    (start_width + x - start_x).clamp(theme::COLUMN_MIN_WIDTH, theme::COLUMN_MAX_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::resized_width;
    use crate::theme::{COLUMN_MAX_WIDTH, COLUMN_MIN_WIDTH};

    #[test]
    fn width_follows_drag_within_limits() {
        assert_eq!(resized_width(230., 500., 550.), 280.);
        assert_eq!(resized_width(230., 500., 0.), COLUMN_MIN_WIDTH);
        assert_eq!(resized_width(230., 500., 5000.), COLUMN_MAX_WIDTH);
    }
}

impl Browser {
    /// 列の境界のドラッグを追う。ウィンドウの外でボタンを離した時は、次の移動で終える。
    pub fn drag_column(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(resize) = self.resizing else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.resizing = None;
            return;
        }
        let width = resized_width(resize.start_width, resize.start_x, event.position.x.into());
        if let Some(column) = self.columns.get_mut(resize.column) {
            column.width = width;
            cx.notify();
        }
    }

    /// 列の余白を押した時は、その列の選択を解いて右の列を閉じる。
    fn clear_column(&mut self, column: usize, cx: &mut Context<Self>) {
        if let Some(current) = self.columns.get_mut(column) {
            current.selected = None;
            self.columns.truncate(column + 1);
            self.preview = None;
            cx.notify();
        }
    }

    /// 列の項目を選ぶ。右の列を閉じ、フォルダなら新しい列を開く。
    fn select_in_column(&mut self, column: usize, name: OsString, entry: Entry, event: &ClickEvent, cx: &mut Context<Self>) {
        let Some(current) = self.columns.get_mut(column) else { return };
        current.selected = Some(name.clone());
        let folder = current.pane.read(cx).state.path.clone();
        let path = folder.join(&name);
        if event.click_count() == 2 && entry.is_dir {
            self.navigate(path, cx);
            return;
        }
        self.columns.truncate(column + 1);
        if entry.is_dir {
            self.preview = None;
            let pane = self.new_pane(path, cx);
            let width = self.settings.default_column_width;
            self.columns.push(Column { pane, scroll: UniformListScrollHandle::new(), selected: None, width });
        } else {
            self.preview = Some((folder, entry));
        }
        // 右端の新しい列が見えるよう、最後まで送る。範囲は描画の時に収められる
        self.columns_scroll.set_offset(Point::new(px(-1_000_000.), px(0.)));
        cx.notify();
    }

    fn column_rows(&mut self, column: usize, range: Range<usize>, cx: &mut Context<Self>) -> Vec<impl IntoElement + use<>> {
        let Some(current) = self.columns.get(column) else { return Vec::new() };
        let selected = current.selected.clone();
        let (selected_background, selected_text) = self.selection_colors();
        let (folder, entries): (PathBuf, Vec<Option<Entry>>) = {
            let state = &current.pane.read(cx).state;
            (state.path.clone(), range.clone().map(|ix| state.entries.get(ix).cloned()).collect())
        };
        range
            .zip(entries)
            .filter_map(|(ix, entry)| {
                let entry = entry?;
                let active = selected.as_deref() == Some(entry.os_name());
                let background = match (active, ix % 2 == 1) {
                    (true, _) => selected_background,
                    (false, true) => theme::STRIPE,
                    (false, false) => theme::BACKGROUND,
                };
                let icon = self.icons.update(cx, |icons, _| icons.get(&folder, &entry, SMALL));
                let name = entry.os_name().to_os_string();
                let is_dir = entry.is_dir;
                let label = entry.name.clone();
                let icon = self.icon(icon, is_dir, theme::ROW_ICON);
                Some(
                    div()
                        .id(ix)
                        .flex()
                        .items_center()
                        .gap_1()
                        .w_full()
                        .h(px(theme::ROW_HEIGHT))
                        .px_2()
                        .bg(rgb(background))
                        .text_color(rgb(if active { selected_text } else { theme::TEXT }))
                        .when(!active, |row| row.hover(|s| s.bg(rgb(theme::HOVER))))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, event, _, cx| {
                            this.select_in_column(column, name.clone(), entry.clone(), event, cx)
                        }))
                        .child(icon)
                        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(label))
                        .when(is_dir, |row| row.child(div().flex_none().text_size(px(theme::SMALL_TEXT_SIZE)).child("›"))),
                )
            })
            .collect()
    }

    fn column_list(&self, column: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = &self.columns[column];
        let state = &current.pane.read(cx).state;
        let count = state.entries.len();
        let error = state.error.clone();
        let scroll = current.scroll.clone();
        let body = match error {
            Some(message) => div().p_2().text_color(rgb(theme::SUB_TEXT)).child(message).into_any_element(),
            None => uniform_list(
                ("column", column),
                count,
                cx.processor(move |this, range, _: &mut Window, cx| this.column_rows(column, range, cx)),
            )
            .track_scroll(scroll)
            .size_full()
            .into_any_element(),
        };
        let width = current.width;
        let handle = div()
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
                    this.resizing = Some(ColumnResize { column, start_x: event.position.x.into(), start_width: width });
                    cx.stop_propagation();
                }),
            );
        div()
            .relative()
            .flex_none()
            .w(px(width))
            .h_full()
            .border_r_1()
            .border_color(rgb(theme::LINE))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| this.clear_column(column, cx)))
            .child(body)
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
        strip
    }
}
