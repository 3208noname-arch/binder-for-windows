use crate::browser::Browser;
use crate::icon_cache::LARGE;
use crate::list_tree::{Flattened, Row};
use crate::theme;
use filer_core::{Date, Entry, GroupKey};
use gpui::{AnyElement, Context, MouseButton, Window, div, list, prelude::*, px, rgb};
use std::path::Path;
use std::rc::Rc;

/// アイコン表示の行の並びと、それを作った時の条件。
pub struct GridData {
    version: u64,
    group: GroupKey,
    per_row: usize,
    today: Date,
    rows: Rc<Vec<GridRow>>,
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
    fn grid_cell(&self, ix: usize, folder: &Path, entry: &Entry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let path = folder.join(entry.os_name());
        let selected = self.selection.contains(&path);
        let whole_cell = self.settings.grid_selection_matches_hover;
        let (selected_background, selected_text) = self.selection_colors();
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
            .when(selected, |l| l.text_color(rgb(selected_text)))
            .when(selected && !whole_cell, |l| l.bg(rgb(selected_background)))
            .child(self.name_or_input(&path, entry));
        let cell = div()
            .id(("cell", ix))
            .when(self.is_cut(&path), |cell| cell.opacity(theme::CUT_OPACITY))
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_1()
            .p_1()
            .w(px(theme::GRID_CELL_WIDTH))
            .rounded_md()
            .when(!selected, |c| c.hover(|s| s.bg(rgb(theme::HOVER))))
            .when(selected && whole_cell, |c| c.bg(rgb(selected_background)))
            .child(self.icon(icon, is_dir, theme::GRID_ICON))
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
                div().id(index).flex().w_full().h(px(theme::GRID_ROW_HEIGHT)).px_2().children(cells).into_any_element()
            }
            None => div().into_any_element(),
        }
    }

    /// 中身か横の個数が変わった時だけ、行を作り直す。スクロールの位置は保つ。
    fn refresh_grid(&mut self, per_row: usize, cx: &Context<Self>) {
        let pane = self.pane.read(cx);
        let group = self.view.group;
        let today = filer_shell::today();
        let fresh = self.grid_data.as_ref().is_some_and(|data| {
            data.version == pane.version && data.group == group && data.per_row == per_row && data.today == today
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
        self.grid_data = Some(GridData { version, group, per_row, today, rows });
    }

    pub fn render_icons(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let width: f32 = (window.viewport_size().width - px(theme::SIDEBAR_WIDTH + 16.)).into();
        let per_row = ((width / theme::GRID_CELL_WIDTH) as usize).max(1);
        self.grid_per_row = per_row;
        self.refresh_grid(per_row, cx);
        div()
            .flex_1()
            .min_h_0()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                this.commit_rename(window, cx);
                this.clear_selection(cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| this.folder_menu(cx)))
            .child(
                list(
                    self.grid_list.clone(),
                    cx.processor(|this, index: usize, _: &mut Window, cx| this.grid_row(index, cx)),
                )
                .size_full(),
            )
    }
}
