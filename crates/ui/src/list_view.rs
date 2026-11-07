use crate::browser::{Browser, HeaderDrag, Menu, Resize, TreeKey};
use crate::folder_pane::tree_for;
use crate::icon_cache::SMALL;
use crate::list_tree::{Flattened, Row};
use crate::scrollbar::ScrollTarget;
use crate::theme;
use filer_core::{Entry, Field, fit_width, format_size, kind_of};
use filer_shell::{format_time, today};
use gpui::{
    AnyElement, Context, CursorStyle, MouseButton, MouseDownEvent, MouseMoveEvent, Window, div,
    prelude::*, px, rgb, uniform_list,
};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

impl Browser {
    /// グループの見出しの行。押すとそのグループを全部選ぶ。
    pub fn group_header(&self, id: usize, ordinal: usize, label: &str, count: usize, column: Option<usize>, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .h(px(theme::ROW_HEIGHT))
            .px_3()
            .border_t_1()
            .border_b_1()
            .border_color(rgb(theme::LINE))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(rgb(theme::HOVER)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| this.select_group(ordinal, column, window, cx)))
            .child(label.to_string())
            .child(div().text_color(rgb(theme::SUB_TEXT)).child(format!("（{count}）")))
            .into_any_element()
    }

    /// 今の行の並び。開いているフォルダの子を親の下へ展開する。中身が変わるまでは作り直さない。
    pub fn tree(&mut self, cx: &Context<Self>) -> Rc<Flattened> {
        let root = self.pane.read(cx);
        let today = today();
        let key = TreeKey {
            root_version: root.version,
            expanded_version: self.expanded.values().map(|pane| pane.read(cx).version).sum(),
            expanded_count: self.expanded.len(),
            group: self.view.group,
            today,
        };
        if let Some((cached, tree)) = &self.tree_cache
            && *cached == key
        {
            return tree.clone();
        }
        let mut lists: HashMap<PathBuf, &[Entry]> = HashMap::new();
        lists.insert(root.state.path.clone(), &root.state.entries);
        for (path, pane) in &self.expanded {
            lists.insert(path.clone(), &pane.read(cx).state.entries);
        }
        let tree = Rc::new(tree_for(root, &lists, self.view.group, today));
        self.tree_cache = Some((key, tree.clone()));
        tree
    }

    fn list_entry(&self, folder: &Path, index: usize, cx: &Context<Self>) -> Option<Entry> {
        let root = self.pane.read(cx);
        let pane = if root.state.path == folder { root } else { self.expanded.get(folder)?.read(cx) };
        pane.state.entries.get(index).cloned()
    }

    fn toggle_expanded(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.expanded.remove(&path).is_none() {
            let pane = self.new_pane(path.clone(), self.view.order, false, cx);
            self.expanded.insert(path, pane);
        } else {
            // 閉じたフォルダの中で開いていた分も閉じる
            self.expanded.retain(|key, _| !key.starts_with(&path));
        }
        cx.notify();
    }

    /// 1つの項目のセルの文字列。
    fn field_text(&mut self, field: Field, folder: &Path, entry: &Entry, cx: &mut Context<Self>) -> String {
        let properties = || {
            let root = self.pane.read(cx);
            (root.state.path == folder).then(|| root.properties.get(entry.os_name()).cloned()).flatten()
        };
        match field {
            Field::Name => entry.name.clone(),
            Field::Modified => format_time(entry.modified),
            Field::Created => format_time(entry.created),
            Field::Size if !entry.is_dir => format_size(entry.size),
            Field::Size if entry.is_link => "--".to_string(),
            Field::Size => {
                let path = folder.join(entry.os_name());
                self.sizes.update(cx, |sizes, _| sizes.get(&path)).map_or("--".to_string(), format_size)
            }
            Field::Kind => kind_of(entry),
            Field::Author => properties().map(|p| p.author).unwrap_or_default(),
            Field::Tags => properties().map(|p| p.tags).unwrap_or_default(),
            Field::Title => properties().map(|p| p.title).unwrap_or_default(),
        }
    }

    /// 項目の境界のダブルクリック。いちばん長い中身に合わせる。
    fn fit_field(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(column) = self.view.columns.get(index).copied() else { return };
        let width = if column.field == Field::Name {
            let root = self.pane.read(cx);
            let lists = std::iter::once(&root.state.entries)
                .chain(self.expanded.values().map(|pane| &pane.read(cx).state.entries));
            let names = lists.flatten().map(|entry| entry.name.as_str());
            fit_width(names, theme::TEXT_SIZE, theme::NAME_FIT_PADDING)
        } else {
            let folder = self.pane.read(cx).state.path.clone();
            let entries = self.pane.read(cx).state.entries.clone();
            let known_size = |entry: &Entry, this: &Self, cx: &Context<Self>| {
                let size = this.sizes.read(cx).peek(&folder.join(entry.os_name()));
                size.map_or("--".to_string(), format_size)
            };
            // 幅を測るためだけに、全フォルダのサイズ計算を始めないようにする
            let texts: Vec<String> = entries
                .iter()
                .map(|entry| match column.field {
                    Field::Size if entry.is_dir => known_size(entry, self, cx),
                    field => self.field_text(field, &folder, entry, cx),
                })
                .collect();
            fit_width(texts.iter().map(String::as_str).chain([column.field.label()]), theme::TEXT_SIZE, theme::FIELD_FIT_PADDING)
        };
        self.update_view(cx, |view| view.resize_column(index, width));
    }

    pub fn drag_resize(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let x: f32 = event.position.x.into();
        match self.resizing {
            Some(Resize::Field { index, start_x, start_width }) => {
                self.view.resize_column(index, start_width + x - start_x);
                self.columns[0].view = self.view.clone();
                cx.notify();
            }
            Some(Resize::Column { column, start_x, start_width }) => self.resize_column_view(column, start_width + x - start_x, cx),
            None => {}
        }
    }

    /// 窓の座標から、見出しのどの位置かを求める。
    fn header_position(&self, x: f32) -> usize {
        let offset: f32 = self.list_columns_scroll.offset().x.into();
        let mut edge = theme::SIDEBAR_WIDTH + theme::ROW_SIDE_PADDING + offset;
        for (index, column) in self.view.columns.iter().enumerate() {
            edge += column.width + theme::FIELD_GAP;
            if x < edge {
                return index;
            }
        }
        self.view.columns.len().saturating_sub(1)
    }

    pub fn drag_header(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let x: f32 = event.position.x.into();
        let target = self.header_position(x);
        if let Some(drag) = &mut self.header_drag
            && (drag.target.is_some() || (x - drag.start_x).abs() > theme::DRAG_THRESHOLD)
        {
            drag.target = Some(target);
            cx.notify();
        }
    }

    pub fn drop_header(&mut self, drag: HeaderDrag, cx: &mut Context<Self>) {
        match drag.target {
            Some(target) => self.update_view(cx, |view| view.move_column(drag.index, target)),
            None => {
                let key = self.view.columns.get(drag.index).and_then(|c| c.field.sort_key());
                if let Some(key) = key {
                    self.toggle_sort(key, cx);
                }
            }
        }
    }

    fn header_cell(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let column = self.view.columns[index];
        let order = self.view.order;
        let mark = match (column.field.sort_key() == Some(order.key), order.descending) {
            (false, _) => "",
            (true, false) => " ▲",
            (true, true) => " ▼",
        };
        let dropping = self.header_drag.is_some_and(|drag| drag.target == Some(index) && drag.index != index);
        let start_width = column.width;
        let handle = div()
            .id(("field-handle", index))
            .absolute()
            .top_0()
            .right(px(-theme::COLUMN_HANDLE_WIDTH))
            .h_full()
            .w(px(theme::COLUMN_HANDLE_WIDTH))
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(|s| s.bg(rgb(theme::LINE)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        this.fit_field(index, cx);
                    } else {
                        this.resizing = Some(Resize::Field { index, start_x: event.position.x.into(), start_width });
                    }
                }),
            );
        div()
            .id(("field-header", index))
            .relative()
            .flex_none()
            .h_full()
            .flex()
            .items_center()
            .w(px(column.width))
            .cursor(CursorStyle::PointingHand)
            .when(dropping, |cell| cell.bg(rgb(theme::HOVER)))
            .when(column.field == Field::Name, |cell| cell.pl(px(theme::DISCLOSURE_WIDTH + theme::ROW_ICON + theme::FIELD_GAP)))
            .child(div().overflow_hidden().whitespace_nowrap().child(format!("{}{mark}", column.field.label())))
            .child(handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.menu = None;
                    this.header_drag = Some(HeaderDrag { index, start_x: event.position.x.into(), target: None });
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.menu = Some(Menu::Fields(event.position));
                    cx.notify();
                }),
            )
    }

    /// 名前の表示。編集中なら入力欄を出す。
    pub fn name_or_input(&self, path: &Path, entry: &Entry) -> AnyElement {
        match &self.renaming {
            Some(renaming) if renaming.path == path => div().flex_1().min_w_0().child(renaming.input.clone()).into_any_element(),
            _ => div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(entry.name.clone()).into_any_element(),
        }
    }

    fn name_cell(&mut self, depth: u32, folder: &Path, entry: &Entry, path: &Path, sub_text: u32, cx: &mut Context<Self>) -> AnyElement {
        let is_dir = entry.is_dir;
        let expanded = self.expanded.get(path);
        let note = expanded.and_then(|pane| pane.read(cx).state.note());
        let expanded = expanded.is_some();
        let icon = self.icons.update(cx, |icons, _| icons.get(folder, entry, SMALL));
        let slot = div().flex_none().w(px(theme::DISCLOSURE_WIDTH)).text_color(rgb(sub_text));
        let disclosure = if is_dir {
            let target = path.to_path_buf();
            slot.id(gpui::ElementId::Name(format!("open-{}", path.display()).into()))
                .cursor(CursorStyle::PointingHand)
                .child(if expanded { "▾" } else { "▸" })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_expanded(target.clone(), cx);
                }))
                .into_any_element()
        } else {
            slot.into_any_element()
        };
        let renaming = self.renaming.as_ref().is_some_and(|renaming| renaming.path == path);
        let label = div()
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .when(renaming, |label| label.flex_1())
            .child(self.item_icon(entry, icon, theme::ROW_ICON))
            .child(self.name_or_input(path, entry));
        let label = self.grab_zone(label, cx);
        div()
            .flex()
            .size_full()
            .items_center()
            .gap_1()
            .when(self.is_dimmed(path, entry), |cell| cell.opacity(theme::CUT_OPACITY))
            .pl(px(depth as f32 * theme::INDENT))
            .child(disclosure)
            .child(label)
            .children(note.map(|note| div().flex_none().text_color(rgb(sub_text)).child(note)))
            .into_any_element()
    }

    fn list_row(&mut self, ix: usize, depth: u32, folder: &Path, entry: &Entry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let path = folder.join(entry.os_name());
        let selected = self.selection.contains(&path);
        let (selected_background, selected_text) = self.selection_colors();
        let background = match (selected, ix % 2 == 1) {
            (true, _) => selected_background,
            (false, true) => theme::STRIPE,
            (false, false) => theme::BACKGROUND,
        };
        let sub_text = if selected { selected_text } else { theme::SUB_TEXT };
        let text = if selected { selected_text } else { theme::TEXT };
        let is_dir = entry.is_dir;
        let columns = self.view.columns.clone();
        let cells: Vec<_> = columns
            .iter()
            .map(|column| {
                let cell = div().flex_none().h_full().flex().items_center().w(px(column.width)).overflow_hidden();
                if column.field == Field::Name {
                    cell.child(self.name_cell(depth, folder, entry, &path, sub_text, cx))
                } else {
                    let content = self.field_text(column.field, folder, entry, cx);
                    cell.text_color(rgb(sub_text)).whitespace_nowrap().child(content)
                }
            })
            .collect();
        let row = div()
            .id(ix)
            .flex()
            .items_center()
            .w_full()
            .h(px(theme::ROW_HEIGHT))
            .px(px(theme::ROW_SIDE_PADDING))
            .gap(px(theme::FIELD_GAP))
            .bg(rgb(background))
            .text_color(rgb(text))
            .when(!selected, |row| row.hover(|s| s.bg(rgb(theme::HOVER))))
            .children(cells);
        let click_path = path.clone();
        self.item_mouse(row, path, is_dir, Some(None), cx)
            .on_click(cx.listener(move |this, event, window, cx| this.click_item(click_path.clone(), is_dir, event, window, cx)))
    }

    fn list_rows(&mut self, tree: &Flattened, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        range
            .filter_map(|ix| match tree.rows.get(ix)? {
                Row::Header { label, count } => {
                    let ordinal = tree.rows[..ix].iter().filter(|row| matches!(row, Row::Header { .. })).count();
                    Some(self.group_header(ix, ordinal, label, *count, None, cx))
                }
                Row::Item(row) => {
                    let folder = &tree.folders[row.folder as usize];
                    let entry = self.list_entry(folder, row.entry as usize, cx)?;
                    Some(self.list_row(ix, row.depth, folder, &entry, cx).into_any_element())
                }
            })
            .collect()
    }

    pub fn render_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let cells: Vec<_> = (0..self.view.columns.len()).map(|index| self.header_cell(index, cx)).collect();
        let total: f32 = self.view.columns.iter().map(|c| c.width + theme::FIELD_GAP).sum::<f32>() + theme::ROW_SIDE_PADDING * 2.0;
        let header = div()
            .flex()
            .flex_none()
            .w_full()
            .h(px(theme::ROW_HEIGHT))
            .items_center()
            .px(px(theme::ROW_SIDE_PADDING))
            .gap(px(theme::FIELD_GAP))
            .border_b_1()
            .border_color(rgb(theme::LINE))
            .text_color(rgb(theme::SUB_TEXT))
            .children(cells);
        // 1回の描画で何度も呼ばれるので、行の並びは1回だけ作る
        let tree = self.tree(cx);
        let count = tree.rows.len();
        let vertical_bar = self.scrollbar("list-bar", ScrollTarget::List, true, theme::ROW_HEIGHT, cx);
        let horizontal_bar = self.scrollbar("list-columns-bar", ScrollTarget::ListColumns, false, 0., cx);
        let mut strip = div()
            .id("list-strip")
            .flex_1()
            .min_h_0()
            .overflow_x_scroll()
            .track_scroll(&self.list_columns_scroll)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.commit_rename(window, cx);
                if !event.modifiers.control {
                    this.clear_selection(cx);
                }
                this.suppress_click = false;
                this.begin_band(event.position, None, event.modifiers.control, cx);
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, _, cx| this.folder_menu(cx)))
            .child(
                div().flex().flex_col().h_full().min_w_full().w(px(total)).child(header).child(
                    div().flex_1().min_h_0().child(
                        uniform_list(
                            "list-rows",
                            count,
                            cx.processor(move |this, range, _: &mut Window, cx| this.list_rows(&tree, range, cx)),
                        )
                        .track_scroll(self.list_scroll.clone())
                        .size_full(),
                    ),
                ),
            );
        strip.style().restrict_scroll_to_axis = Some(true);
        div().relative().flex_1().min_h_0().flex().flex_col().child(strip).children(vertical_bar).children(horizontal_bar)
    }
}
