use crate::browser::Browser;
use crate::icon_cache::{HUGE, LARGE};
use crate::scrollbar::ScrollTarget;
use crate::list_tree::{Flattened, Row};
use crate::theme;
use filer_core::{Date, Entry, GroupKey, LongName, text_width};
use gpui::{
    Animation, AnimationExt, AnyElement, Context, MouseButton, MouseDownEvent, Window, div, linear_color_stop,
    linear_gradient, list, prelude::*, px, rgb,
};
use std::path::Path;
use std::time::Duration;
use std::rc::Rc;

/// アイコン表示の行の並びと、それを作った時の条件。
pub struct GridData {
    version: u64,
    group: GroupKey,
    per_row: usize,
    icon_size: f32,
    today: Date,
    pub rows: Rc<Vec<GridRow>>,
}

/// アイコンの大きさからセルの幅と行の高さを決める。
pub fn grid_metrics(icon_size: f32) -> (f32, f32) {
    let label = theme::GRID_LABEL_LINE * theme::GRID_LABEL_LINES;
    (icon_size.max(theme::GRID_MIN_CELL) + theme::GRID_CELL_MARGIN, icon_size + label + theme::GRID_ROW_MARGIN)
}

/// アイコン表示の1行。見出しか、並べるセルの番号。
pub enum GridRow {
    Header { ordinal: usize, label: String, count: usize },
    Cells(Vec<u32>),
}

/// 行の並びを、横に `per_row` 個ずつのセルへ畳む。
pub fn grid_rows_of(tree: &Flattened, per_row: usize) -> Vec<GridRow> {
    let mut rows = Vec::new();
    let mut cells = Vec::new();
    let mut ordinal = 0;
    for row in &tree.rows {
        match row {
            Row::Header { label, count } => {
                if !cells.is_empty() {
                    rows.push(GridRow::Cells(std::mem::take(&mut cells)));
                }
                rows.push(GridRow::Header { ordinal, label: label.clone(), count: *count });
                ordinal += 1;
            }
            Row::Item(item) => {
                cells.push(item.entry);
                if cells.len() == per_row {
                    rows.push(GridRow::Cells(std::mem::take(&mut cells)));
                }
            }
        }
    }
    if !cells.is_empty() {
        rows.push(GridRow::Cells(cells));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list_tree::TreeRow;

    fn item(entry: u32) -> Row {
        Row::Item(TreeRow { depth: 0, folder: 0, entry })
    }

    #[test]
    fn group_starts_a_new_grid_row() {
        let tree = Flattened {
            folders: vec!["r".into()],
            rows: vec![
                Row::Header { label: "a".into(), count: 3 },
                item(0),
                item(1),
                item(2),
                Row::Header { label: "b".into(), count: 1 },
                item(3),
            ],
        };
        let rows = grid_rows_of(&tree, 2);
        let shape: Vec<_> = rows
            .iter()
            .map(|r| match r {
                GridRow::Header { .. } => vec![],
                GridRow::Cells(c) => c.clone(),
            })
            .collect();
        assert_eq!(shape, [vec![], vec![0, 1], vec![2], vec![], vec![3]]);
    }
}

impl Browser {
    /// アイコンの下の名前。長い名前は折り返し、それでも収まらない分は設定に従う。
    fn grid_label(&self, ix: usize, path: &Path, entry: &Entry, width: f32, selected: bool, background: u32, text: u32) -> AnyElement {
        if self.renaming.as_ref().is_some_and(|renaming| renaming.path == path) {
            return self.name_or_input(path, entry);
        }
        let name = entry.name.clone();
        let total = text_width(&name, theme::SMALL_TEXT_SIZE);
        let overflowing = total > width * theme::GRID_LABEL_LINES;
        let base = div()
            .relative()
            .w_full()
            .h(px(theme::GRID_LABEL_LINE * theme::GRID_LABEL_LINES))
            .overflow_hidden()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .line_height(px(theme::GRID_LABEL_LINE))
            .when(selected, |label| label.text_color(rgb(text)));
        let hovered = self.hovered_cell.as_deref() == Some(path);
        if overflowing && hovered && self.settings.long_name == LongName::Scroll {
            let distance = total - width;
            let seconds = (distance / theme::MARQUEE_SPEED).max(1.0);
            let moving = div().whitespace_nowrap().child(name).with_animation(
                ("marquee", ix),
                Animation::new(Duration::from_secs_f32(seconds)).repeat(),
                move |line, progress| line.ml(px(-distance * progress)),
            );
            return base.child(moving).into_any_element();
        }
        let color = gpui::Rgba::from(rgb(background));
        let fade = linear_gradient(180., linear_color_stop(gpui::Rgba { a: 0., ..color }, 0.), linear_color_stop(color, 1.));
        base.child(div().w_full().text_center().child(name))
            .when(overflowing, |label| {
                label.child(div().absolute().left_0().right_0().bottom_0().h(px(theme::GRID_LABEL_LINE)).bg(fade))
            })
            .into_any_element()
    }

    fn grid_cell(&self, ix: usize, folder: &Path, entry: &Entry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let path = folder.join(entry.os_name());
        let selected = self.selection.contains(&path);
        let whole_cell = self.settings.grid_selection_matches_hover;
        let (selected_background, selected_text) = self.selection_colors();
        let is_dir = entry.is_dir;
        let size = self.view.icon_size;
        let (cell_width, _) = grid_metrics(size);
        let bucket = if size > LARGE as f32 { HUGE } else { LARGE };
        let icon = self.icons.update(cx, |icons, _| icons.get(folder, entry, bucket));
        let filled = selected && whole_cell;
        let background = if filled { selected_background } else { theme::BACKGROUND };
        let label_width = cell_width - theme::GRID_LABEL_PADDING;
        let label = self.grid_label(ix, &path, entry, label_width, selected, background, selected_text);
        let label_path = path.clone();
        let cell = div()
            .id(("cell", ix))
            .when(self.is_cut(&path), |cell| cell.opacity(theme::CUT_OPACITY))
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_1()
            .p_1()
            .w(px(cell_width))
            .rounded_md()
            .when(!selected, |c| c.hover(|s| s.bg(rgb(theme::HOVER))))
            .when(filled, |c| c.bg(rgb(selected_background)))
            .when(selected && !whole_cell, |c| c.bg(rgb(theme::HOVER)))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let current = this.hovered_cell.as_deref() == Some(label_path.as_path());
                if *hovered {
                    this.hovered_cell = Some(label_path.clone());
                    cx.notify();
                } else if current {
                    this.hovered_cell = None;
                    cx.notify();
                }
            }))
            .child(self.icon(icon, is_dir, size))
            .child(label);
        let click_path = path.clone();
        self.item_mouse(cell, path, is_dir, cx)
            .on_click(cx.listener(move |this, event, window, cx| this.click_item(click_path.clone(), is_dir, event, window, cx)))
    }

    fn grid_row(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(data) = self.grid_data.as_ref().map(|data| data.rows.clone()) else {
            return div().into_any_element();
        };
        let folder = self.pane.read(cx).state.path.clone();
        match data.get(index) {
            Some(GridRow::Header { ordinal, label, count }) => self.group_header(index, *ordinal, label, *count, None, cx),
            Some(GridRow::Cells(cells)) => {
                let entries: Vec<(usize, Entry)> = {
                    let state = &self.pane.read(cx).state;
                    cells.iter().filter_map(|&i| Some((i as usize, state.entries.get(i as usize)?.clone()))).collect()
                };
                let cells: Vec<_> = entries.iter().map(|(ix, entry)| self.grid_cell(*ix, &folder, entry, cx)).collect();
                let (_, row_height) = grid_metrics(self.view.icon_size);
                div().id(index).flex().w_full().h(px(row_height)).px(px(theme::GRID_SIDE_PADDING)).children(cells).into_any_element()
            }
            None => div().into_any_element(),
        }
    }

    /// 中身か横の個数が変わった時だけ、行を作り直す。スクロールの位置は保つ。
    fn refresh_grid(&mut self, per_row: usize, cx: &Context<Self>) {
        let pane = self.pane.read(cx);
        let group = self.view.group;
        let today = filer_shell::today();
        let icon_size = self.view.icon_size;
        let fresh = self.grid_data.as_ref().is_some_and(|data| {
            data.version == pane.version
                && data.group == group
                && data.per_row == per_row
                && data.icon_size == icon_size
                && data.today == today
        });
        if fresh {
            return;
        }
        let (_, tree) = Self::plain_tree(pane, group);
        let rows = Rc::new(grid_rows_of(&tree, per_row));
        let version = pane.version;
        let old = self.grid_list.item_count();
        let top = self.grid_list.logical_scroll_top();
        self.grid_list.splice(0..old, rows.len());
        self.grid_list.scroll_to(top);
        self.grid_data = Some(GridData { version, group, per_row, icon_size, today, rows });
    }

    pub fn render_icons(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let width: f32 = (window.viewport_size().width - px(theme::SIDEBAR_WIDTH + theme::GRID_SIDE_PADDING * 2.)).into();
        let (cell_width, _) = grid_metrics(self.view.icon_size);
        let per_row = ((width / cell_width) as usize).max(1);
        self.grid_per_row = per_row;
        self.refresh_grid(per_row, cx);
        let bar = self.scrollbar("grid-bar", ScrollTarget::Grid, true, 0., cx);
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.commit_rename(window, cx);
                if !event.modifiers.control {
                    this.clear_selection(cx);
                }
                this.begin_band(event.position, None, event.modifiers.control, cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| this.folder_menu(cx)))
            .child(
                list(
                    self.grid_list.clone(),
                    cx.processor(|this, index: usize, _: &mut Window, cx| this.grid_row(index, cx)),
                )
                .size_full(),
            )
            .children(bar)
    }
}
