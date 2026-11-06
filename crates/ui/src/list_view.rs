use crate::browser::{Browser, TreeKey};
use crate::icon_cache::SMALL;
use crate::list_tree::{Flattened, Row, flatten, flatten_grouped};
use crate::theme;
use filer_core::{
    Date, Entry, Group, GroupKey, Properties, SortKey, date_group, format_size, kind_of, name_group, size_group,
    value_group,
};
use filer_shell::{format_time, local_date, today};
use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Div, MouseButton, Stateful, Window, div, prelude::*, px, rgb,
    uniform_list,
};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn group_header(ix: usize, label: &str, count: usize) -> AnyElement {
    div()
        .id(ix)
        .flex()
        .items_end()
        .gap_2()
        .w_full()
        .h(px(theme::ROW_HEIGHT))
        .px_3()
        .pb_0p5()
        .border_b_1()
        .border_color(rgb(theme::LINE))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(label.to_string())
        .child(div().text_color(rgb(theme::SUB_TEXT)).child(format!("（{count}）")))
        .into_any_element()
}

fn group_of(entry: &Entry, key: GroupKey, properties: Option<&Properties>, today: Date) -> Group {
    let value = |read: fn(&Properties) -> &str| value_group(properties.map_or("", read));
    match key {
        GroupKey::None | GroupKey::Name => name_group(&entry.name),
        GroupKey::Modified => date_group(local_date(entry.modified), today),
        GroupKey::Created => date_group(local_date(entry.created), today),
        GroupKey::Size => size_group(entry),
        GroupKey::Author => value(|p| &p.author),
        GroupKey::Tags => value(|p| &p.tags),
        GroupKey::Title => value(|p| &p.title),
    }
}

impl Browser {
    /// 今の行の並び。開いているフォルダの子を親の下へ展開する。中身が変わるまでは作り直さない。
    fn tree(&mut self, cx: &Context<Self>) -> Rc<Flattened> {
        let root = self.pane.read(cx);
        let today = today();
        let key = TreeKey {
            root_version: root.version,
            expanded_version: self.expanded.values().map(|pane| pane.read(cx).version).sum(),
            expanded_count: self.expanded.len(),
            group: self.group,
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
        let group = self.group;
        let flat = match group {
            GroupKey::None => flatten(&root.state.path, &lists),
            _ => flatten_grouped(&root.state.path, &lists, &|entry| {
                group_of(entry, group, root.properties.get(entry.os_name()), today)
            }),
        };
        let tree = Rc::new(flat);
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
            let pane = self.new_pane(path.clone(), cx);
            self.expanded.insert(path, pane);
        } else {
            // 閉じたフォルダの中で開いていた分も閉じる
            self.expanded.retain(|key, _| !key.starts_with(&path));
        }
        cx.notify();
    }

    /// 行の位置は列挙の途中で動くので、番号ではなくパスで受ける。
    fn click_list_row(&mut self, path: PathBuf, is_dir: bool, event: &ClickEvent, cx: &mut Context<Self>) {
        if event.click_count() == 2 && is_dir {
            self.navigate(path, cx);
            return;
        }
        self.selected = Some(path);
        cx.notify();
    }

    fn header_cell(&self, label: &str, key: SortKey, cx: &mut Context<Self>) -> Stateful<Div> {
        let mark = match (self.order.key == key, self.order.descending) {
            (false, _) => "",
            (true, false) => " ▲",
            (true, true) => " ▼",
        };
        div()
            .id(gpui::ElementId::Name(format!("header-{key:?}").into()))
            .cursor(CursorStyle::PointingHand)
            .child(format!("{label}{mark}"))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_sort(key, cx)))
    }

    fn list_row(&self, ix: usize, depth: u32, folder: &Path, entry: &Entry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let path = folder.join(entry.os_name());
        let selected = self.selected.as_deref() == Some(path.as_path());
        let expanded = self.expanded.get(&path);
        let note = expanded.and_then(|pane| pane.read(cx).state.note());
        let expanded = expanded.is_some();
        let (selected_background, selected_text) = self.selection_colors();
        let background = match (selected, ix % 2 == 1) {
            (true, _) => selected_background,
            (false, true) => theme::STRIPE,
            (false, false) => theme::BACKGROUND,
        };
        let sub_text = if selected { selected_text } else { theme::SUB_TEXT };
        let text = if selected { selected_text } else { theme::TEXT };
        let is_dir = entry.is_dir;
        let size = if is_dir { "--".to_string() } else { format_size(entry.size) };
        let icon = self.icons.update(cx, |icons, _| icons.get(folder, entry, SMALL));
        let slot = div().flex_none().w(px(theme::DISCLOSURE_WIDTH)).text_color(rgb(sub_text));
        let disclosure = if is_dir {
            let target = path.clone();
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
        div()
            .id(ix)
            .flex()
            .items_center()
            .w_full()
            .h(px(theme::ROW_HEIGHT))
            .px_3()
            .gap_2()
            .bg(rgb(background))
            .text_color(rgb(text))
            .when(!selected, |row| row.hover(|s| s.bg(rgb(theme::HOVER))))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, event, _, cx| this.click_list_row(path.clone(), is_dir, event, cx)))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .pl(px(depth as f32 * theme::INDENT))
                    .child(disclosure)
                    .child(self.icon(icon, is_dir, theme::ROW_ICON))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(entry.name.clone()))
                    .children(note.map(|note| div().flex_none().text_color(rgb(sub_text)).child(note))),
            )
            .child(div().flex_none().w(px(theme::DATE_WIDTH)).text_color(rgb(sub_text)).child(format_time(entry.modified)))
            .child(div().flex_none().w(px(theme::SIZE_WIDTH)).text_color(rgb(sub_text)).child(size))
            .child(div().flex_none().w(px(theme::KIND_WIDTH)).text_color(rgb(sub_text)).child(kind_of(entry)))
    }

    fn list_rows(&mut self, tree: &Flattened, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        range
            .filter_map(|ix| match tree.rows.get(ix)? {
                Row::Header { label, count } => Some(group_header(ix, label, *count)),
                Row::Item(row) => {
                    let folder = &tree.folders[row.folder as usize];
                    let entry = self.list_entry(folder, row.entry as usize, cx)?;
                    Some(self.list_row(ix, row.depth, folder, &entry, cx).into_any_element())
                }
            })
            .collect()
    }

    pub fn render_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let header = div()
            .flex()
            .flex_none()
            .w_full()
            .h(px(theme::ROW_HEIGHT))
            .items_center()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(theme::LINE))
            .text_color(rgb(theme::SUB_TEXT))
            .child(div().flex_1().min_w_0().pl(px(theme::DISCLOSURE_WIDTH + theme::ROW_ICON + 8.)).child(self.header_cell("名前", SortKey::Name, cx)))
            .child(div().flex_none().w(px(theme::DATE_WIDTH)).child(self.header_cell("更新日時", SortKey::Modified, cx)))
            .child(div().flex_none().w(px(theme::SIZE_WIDTH)).child(self.header_cell("サイズ", SortKey::Size, cx)))
            .child(div().flex_none().w(px(theme::KIND_WIDTH)).child(self.header_cell("種類", SortKey::Kind, cx)));
        // 1回の描画で何度も呼ばれるので、行の並びは1回だけ作る
        let tree = self.tree(cx);
        let count = tree.rows.len();
        div().flex().flex_col().flex_1().min_h_0().child(header).child(
            div()
                .flex_1()
                .min_h_0()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.clear_selection(cx)))
                .child(
                uniform_list(
                    "list-rows",
                    count,
                    cx.processor(move |this, range, _: &mut Window, cx| this.list_rows(&tree, range, cx)),
                )
                    .track_scroll(self.list_scroll.clone())
                    .size_full(),
            ),
        )
    }
}
