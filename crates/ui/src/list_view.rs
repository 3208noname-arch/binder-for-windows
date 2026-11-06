use crate::browser::Browser;
use crate::icon_cache::SMALL;
use crate::list_tree::{Flattened, flatten};
use crate::theme;
use filer_core::{Entry, SortKey, format_size, kind_of};
use filer_shell::format_time;
use gpui::{ClickEvent, Context, CursorStyle, Div, Stateful, Window, div, prelude::*, px, rgb, uniform_list};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;

impl Browser {
    /// 今の行の並び。開いているフォルダの子を親の下へ展開する。
    fn tree(&self, cx: &Context<Self>) -> Flattened {
        let root = self.pane.read(cx);
        let mut lists: HashMap<PathBuf, &[Entry]> = HashMap::new();
        lists.insert(root.state.path.clone(), &root.state.entries);
        for (path, pane) in &self.expanded {
            lists.insert(path.clone(), &pane.read(cx).state.entries);
        }
        flatten(&root.state.path, &lists)
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
        let background = match (selected, ix % 2 == 1) {
            (true, _) => theme::SELECTED,
            (false, true) => theme::STRIPE,
            (false, false) => theme::BACKGROUND,
        };
        let sub_text = if selected { theme::SELECTED_TEXT } else { theme::SUB_TEXT };
        let text = if selected { theme::SELECTED_TEXT } else { theme::TEXT };
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

    fn list_rows(&mut self, tree: &Flattened, range: Range<usize>, cx: &mut Context<Self>) -> Vec<impl IntoElement + use<>> {
        range
            .filter_map(|ix| {
                let row = tree.rows.get(ix)?;
                let folder = &tree.folders[row.folder as usize];
                let entry = self.list_entry(folder, row.entry as usize, cx)?;
                Some(self.list_row(ix, row.depth, folder, &entry, cx))
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
        let tree = Rc::new(self.tree(cx));
        let count = tree.rows.len();
        div().flex().flex_col().flex_1().min_h_0().child(header).child(
            div().flex_1().min_h_0().child(
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
