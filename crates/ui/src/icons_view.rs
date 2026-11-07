use crate::browser::Browser;
use crate::icon_cache::{HUGE, LARGE};
use crate::scrollbar::ScrollTarget;
use crate::list_tree::{Flattened, Row};
use crate::theme;
use crate::text_input::TextInput;
use filer_core::{Date, Entry, GroupKey, MAX_ICON_SIZE, MIN_ICON_SIZE};
use gpui::{
    Animation, AnimationExt, AnyElement, Context, CursorStyle, Entity, MouseButton, MouseDownEvent, ScrollWheelEvent, TextRun, Window, black,
    deferred, div, font, linear_color_stop, linear_gradient, list, prelude::*, px, rgb,
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

/// 名前を折り返した時の行数。
fn label_lines(name: &str, width: f32, window: &Window) -> f32 {
    let run = TextRun { len: name.len(), font: font(theme::FONT), color: black(), background_color: None, underline: None, strikethrough: None };
    let lines = window.text_system().shape_text(name.to_string().into(), px(theme::SMALL_TEXT_SIZE), &[run], Some(px(width)), None);
    lines.map_or(1, |lines| lines.iter().map(|line| line.wrap_boundaries.len() + 1).sum::<usize>()) as f32
}

/// 色やカーソルなど、名前の見せ方の材料。
struct LabelLook {
    selected: bool,
    highlight: bool,
    fill: u32,
    text: u32,
}

impl Browser {
    /// 名前の編集欄。折り返しは表示と同じ幅にして、隣のセルの下の行まで重ねて出す。
    fn grid_rename(&self, lines: f32, input: Entity<TextInput>) -> AnyElement {
        let width = theme::grid_label_width(self.view.icon_size) + theme::RENAME_SIDE * 2.;
        div()
            .relative()
            .w_full()
            .h(px(theme::GRID_LABEL_LINE * lines.min(theme::GRID_LABEL_LINES)))
            .child(deferred(div().absolute().top_0().left(px(theme::GRID_LABEL_SIDE - theme::RENAME_SIDE)).w(px(width)).child(input)))
            .into_any_element()
    }

    /// アイコンの下の名前。3行に収まらない分は薄く消し、設定によってマウスを乗せると流す。
    fn grid_label(&self, ix: usize, path: &Path, entry: &Entry, look: LabelLook, window: &Window) -> AnyElement {
        let width = theme::grid_label_width(self.view.icon_size);
        let lines = label_lines(&entry.name, width, window);
        if let Some(renaming) = self.renaming.as_ref().filter(|renaming| renaming.path == path) {
            return self.grid_rename(lines, renaming.input.clone());
        }
        let line = theme::GRID_LABEL_LINE;
        let overflowing = lines > theme::GRID_LABEL_LINES;
        let hovered = self.hovered_cell.as_deref() == Some(path);
        let label = div()
            .relative()
            .flex_none()
            .max_w_full()
            .px(px(theme::GRID_LABEL_SIDE))
            .rounded_sm()
            .overflow_hidden()
            .when(look.highlight, |label| label.bg(rgb(look.fill)))
            .when(look.selected, |label| label.text_color(rgb(look.text)))
            .when(overflowing, |label| label.h(px(line * theme::GRID_LABEL_LINES)));
        let text = div().text_center().child(entry.name.clone());
        let label = if overflowing && hovered && self.settings.scroll_long_names {
            // 全文が見えた位置で止める。名前は行で読むものなので、横には動かさない
            let distance = (lines - theme::GRID_LABEL_LINES) * line;
            let travel = distance / theme::MARQUEE_SPEED;
            let total = travel + theme::MARQUEE_PAUSE;
            let wait = theme::MARQUEE_PAUSE / total;
            let animation = Animation::new(Duration::from_secs_f32(total)).with_easing(move |t| ((t - wait) / (1. - wait)).clamp(0., 1.));
            label.child(text.with_animation(("marquee", ix), animation, move |block, progress| block.mt(px(-distance * progress))))
        } else {
            let color = gpui::Rgba::from(rgb(look.fill));
            let fade = linear_gradient(180., linear_color_stop(gpui::Rgba { a: 0., ..color }, 0.), linear_color_stop(color, 1.));
            label.child(text).when(overflowing, |label| label.child(div().absolute().left_0().right_0().bottom_0().h(px(line)).bg(fade)))
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .line_height(px(line))
            .child(label)
            .into_any_element()
    }

    fn grid_cell(&self, ix: usize, folder: &Path, entry: &Entry, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
        let hovered = self.hovered_cell.as_deref() == Some(path.as_path());
        // 全体のハイライトを切った時は、名前だけを選択の色にする
        let highlight = selected && !whole_cell;
        let fill = match (selected, hovered) {
            (true, _) => selected_background,
            (false, true) => theme::HOVER,
            _ => theme::BACKGROUND,
        };
        let look = LabelLook { selected, highlight, fill, text: selected_text };
        let label = self.grid_label(ix, &path, entry, look, window);
        let label_path = path.clone();
        let cell = div()
            .id(("cell", ix))
            .when(self.is_dimmed(&path, entry), |cell| cell.opacity(theme::CUT_OPACITY))
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap_1()
            .p_1()
            .w(px(cell_width))
            .rounded_md()
            .cursor(CursorStyle::Arrow)
            .when(!selected, |c| c.hover(|s| s.bg(rgb(theme::HOVER))))
            .when(filled, |c| c.bg(rgb(selected_background)))
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
            .child(div().flex_none().rounded_md().when(highlight, |box_| box_.bg(rgb(theme::HOVER))).child(self.item_icon(entry, icon, size)))
            .child(label);
        let click_path = path.clone();
        self.item_mouse(cell, path, is_dir, None, cx)
            .on_click(cx.listener(move |this, event, window, cx| this.click_item(click_path.clone(), is_dir, event, window, cx)))
    }

    fn grid_row(&mut self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
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
                let cells: Vec<_> = entries.iter().map(|(ix, entry)| self.grid_cell(*ix, &folder, entry, window, cx)).collect();
                let (_, row_height) = grid_metrics(self.view.icon_size);
                div()
                    .id(index)
                    .flex()
                    .items_start()
                    .gap(px(theme::GRID_CELL_GAP))
                    .w_full()
                    .h(px(row_height))
                    .px(px(theme::GRID_SIDE_PADDING))
                    .children(cells)
                    .into_any_element()
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
        let per_row = ((width / (cell_width + theme::GRID_CELL_GAP)) as usize).max(1);
        self.grid_per_row = per_row;
        self.refresh_grid(per_row, cx);
        let bar = self.scrollbar("grid-bar", ScrollTarget::Grid, true, 0., cx);
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .cursor(CursorStyle::Crosshair)
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                if event.modifiers.control {
                    let notches = f32::from(event.delta.pixel_delta(px(theme::ROW_HEIGHT)).y).signum();
                    let size = (this.view.icon_size + notches * theme::ICON_SIZE_STEP).clamp(MIN_ICON_SIZE, MAX_ICON_SIZE);
                    this.update_view(cx, |view| view.icon_size = size);
                }
            }))
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
                    cx.processor(|this, index: usize, window: &mut Window, cx| this.grid_row(index, window, cx)),
                )
                .size_full(),
            )
            .children(bar)
    }
}
