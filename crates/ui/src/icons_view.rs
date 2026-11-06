use crate::browser::Browser;
use crate::icon_cache::LARGE;
use crate::theme;
use filer_core::Entry;
use gpui::{ClickEvent, Context, Window, div, prelude::*, px, rgb, uniform_list};
use std::ops::Range;
use std::path::PathBuf;

impl Browser {
    fn click_cell(&mut self, path: PathBuf, is_dir: bool, event: &ClickEvent, cx: &mut Context<Self>) {
        if event.click_count() == 2 && is_dir {
            self.navigate(path, cx);
            return;
        }
        self.selected = Some(path);
        cx.notify();
    }

    fn grid_cell(&self, ix: usize, folder: &PathBuf, entry: &Entry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let path = folder.join(entry.os_name());
        let selected = self.selected.as_deref() == Some(path.as_path());
        let is_dir = entry.is_dir;
        let icon = self.icons.update(cx, |icons, _| icons.get(folder, entry, LARGE));
        let label = div()
            .max_w_full()
            .px_1()
            .rounded_sm()
            .overflow_hidden()
            .text_center()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .line_clamp(2)
            .when(selected, |l| l.bg(rgb(theme::SELECTED)).text_color(rgb(theme::SELECTED_TEXT)))
            .child(entry.name.clone());
        div()
            .id(("cell", ix))
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_1()
            .p_1()
            .w(px(theme::GRID_CELL_WIDTH))
            .rounded_md()
            .when(!selected, |c| c.hover(|s| s.bg(rgb(theme::HOVER))))
            .on_click(cx.listener(move |this, event, _, cx| this.click_cell(path.clone(), is_dir, event, cx)))
            .child(self.icon(icon, is_dir, theme::GRID_ICON))
            .child(label)
    }

    fn grid_rows(&mut self, per_row: usize, range: Range<usize>, cx: &mut Context<Self>) -> Vec<impl IntoElement + use<>> {
        let (folder, entries): (PathBuf, Vec<(usize, Entry)>) = {
            let state = &self.pane.read(cx).state;
            let first = range.start * per_row;
            let last = (range.end * per_row).min(state.entries.len());
            (state.path.clone(), (first..last).map(|i| (i, state.entries[i].clone())).collect())
        };
        entries
            .chunks(per_row)
            .enumerate()
            .map(|(row, cells)| {
                let cells: Vec<_> = cells.iter().map(|(ix, entry)| self.grid_cell(*ix, &folder, entry, cx)).collect();
                div()
                    .id(range.start + row)
                    .flex()
                    .w_full()
                    .h(px(theme::GRID_ROW_HEIGHT))
                    .px_2()
                    .children(cells)
            })
            .collect()
    }

    pub fn render_icons(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let width: f32 = (window.viewport_size().width - px(theme::SIDEBAR_WIDTH + 16.)).into();
        let per_row = ((width / theme::GRID_CELL_WIDTH) as usize).max(1);
        let rows = self.pane.read(cx).state.entries.len().div_ceil(per_row);
        div().flex_1().min_h_0().child(
            uniform_list(
                "grid",
                rows,
                cx.processor(move |this, range, _: &mut Window, cx| this.grid_rows(per_row, range, cx)),
            )
            .track_scroll(self.grid_scroll.clone())
            .size_full(),
        )
    }
}
