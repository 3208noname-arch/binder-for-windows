use crate::list_state::{ListState, coalesce};
use crate::theme;
use filer_core::{Entry, SortKey, SortOrder, format_size, kind_of};
use filer_shell::{ListJob, format_time, start_list};
use futures::StreamExt;
use gpui::{
    ClickEvent, Context, CursorStyle, Div, Stateful, Task, UniformListScrollHandle, Window, div,
    prelude::*, px, rgb, uniform_list,
};
use std::ffi::OsString;
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct ListView {
    state: ListState,
    selected: Option<OsString>,
    scroll: UniformListScrollHandle,
    job: Option<ListJob>,
    loader: Option<Task<()>>,
    measure: Option<Measure>,
}

struct Measure {
    start: Instant,
    first_row_seen: bool,
    reopened: bool,
}

const MERGE_INTERVAL: Duration = Duration::from_millis(8);

/// 測定用に順番を崩す。Rustの並べ替えは整列済みの入力で速いので、そのままでは測れない。
fn shuffle(entries: &mut [Entry]) {
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    for i in (1..entries.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        entries.swap(i, (seed % (i as u64 + 1)) as usize);
    }
}

impl ListView {
    pub fn new(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            state: ListState::new(path.clone()),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            job: None,
            loader: None,
            measure: None,
        };
        view.open(path, cx);
        view
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let generation = self.state.open(path.clone());
        self.selected = None;
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        self.job = Some(start_list(path, move |event| {
            let _ = tx.unbounded_send(event);
        }));
        self.loader = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut events = vec![first];
                while let Ok(event) = rx.try_recv() {
                    events.push(event);
                }
                let applied = this.update(cx, |view, cx| {
                    for event in coalesce(events) {
                        view.state.apply(generation, event);
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
                // 描画の順番を譲り、その間に届いた分をまとめて併合する
                cx.background_executor().timer(MERGE_INTERVAL).await;
            }
            let _ = this.update(cx, |view, cx| view.finish_measure(cx));
        }));
        cx.notify();
    }

    /// `--measure` の時だけ呼ぶ。測定の開始時刻を覚える。
    pub fn measure_from(&mut self, start: Instant) {
        self.measure = Some(Measure { start, first_row_seen: false, reopened: false });
    }

    fn finish_measure(&mut self, cx: &mut Context<Self>) {
        let Some(measure) = &mut self.measure else { return };
        println!("all_entries_ms={} count={}", measure.start.elapsed().as_millis(), self.state.entries.len());
        if measure.reopened {
            cx.quit();
            return;
        }
        for key in [SortKey::Modified, SortKey::Size, SortKey::Kind, SortKey::Name] {
            shuffle(&mut self.state.entries);
            let started = Instant::now();
            self.state.set_order(SortOrder { key, descending: false });
            println!("sort_{key:?}_ms={}", started.elapsed().as_millis());
        }
        // 2回目は、起動の時間を含まないフォルダ移動の時間を測る
        *measure = Measure { start: Instant::now(), first_row_seen: false, reopened: true };
        let path = self.state.path.clone();
        self.open(path, cx);
    }

    fn open_parent(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = self.state.path.parent() {
            let parent = parent.to_path_buf();
            self.open(parent, cx);
        }
    }

    fn toggle_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        let current = self.state.order;
        let descending = current.key == key && !current.descending;
        self.state.set_order(SortOrder { key, descending });
        cx.notify();
    }

    /// 列挙の途中でも行の位置は動くので、番号ではなく名前で受ける。
    fn click_row(&mut self, name: OsString, is_dir: bool, event: &ClickEvent, cx: &mut Context<Self>) {
        if event.click_count() == 2 && is_dir {
            let path = self.state.path.join(&name);
            self.open(path, cx);
            return;
        }
        self.selected = Some(name);
        cx.notify();
    }

    fn header_cell(&self, label: &str, key: SortKey, cx: &mut Context<Self>) -> Stateful<Div> {
        let order = self.state.order;
        let mark = match (order.key == key, order.descending) {
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

    fn row(&self, ix: usize, entry: &Entry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let selected = self.selected.as_deref() == Some(entry.os_name());
        let (name, is_dir) = (entry.os_name().to_os_string(), entry.is_dir);
        let background = match (selected, ix % 2 == 1) {
            (true, _) => theme::SELECTED,
            (false, true) => theme::STRIPE,
            (false, false) => theme::BACKGROUND,
        };
        let sub_text = if selected { theme::SELECTED_TEXT } else { theme::SUB_TEXT };
        let text = if selected { theme::SELECTED_TEXT } else { theme::TEXT };
        let icon = if entry.is_dir { theme::FOLDER } else { theme::FILE };
        let size = if entry.is_dir { "--".to_string() } else { format_size(entry.size) };
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
            .on_click(cx.listener(move |this, event, _, cx| this.click_row(name.clone(), is_dir, event, cx)))
            .child(div().flex_none().size(px(14.)).rounded_sm().bg(rgb(icon)))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(entry.name.clone()))
            .child(div().flex_none().w(px(theme::DATE_WIDTH)).text_color(rgb(sub_text)).child(format_time(entry.modified)))
            .child(div().flex_none().w(px(theme::SIZE_WIDTH)).text_color(rgb(sub_text)).child(size))
            .child(div().flex_none().w(px(theme::KIND_WIDTH)).text_color(rgb(sub_text)).child(kind_of(entry)))
    }

    fn rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<impl IntoElement + use<>> {
        let entries = std::mem::take(&mut self.state.entries);
        let rows = range.map(|ix| self.row(ix, &entries[ix], cx)).collect();
        self.state.entries = entries;
        rows
    }
}

impl Render for ListView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(measure) = &mut self.measure
            && !measure.first_row_seen
            && !self.state.entries.is_empty()
        {
            measure.first_row_seen = true;
            let label = if measure.reopened { "reopen_first_row_ms" } else { "first_row_ms" };
            println!("{label}={}", measure.start.elapsed().as_millis());
        }
        let title =self.state.path.file_name().map_or_else(
            || self.state.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(theme::TOOLBAR_HEIGHT))
            .px_3()
            .bg(rgb(theme::BAR))
            .border_b_1()
            .border_color(rgb(theme::LINE))
            .child(
                div()
                    .id("parent")
                    .px_2()
                    .rounded_md()
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(rgb(theme::HOVER)))
                    .child("↑")
                    .on_click(cx.listener(Self::open_parent)),
            )
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(title));
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
            .child(div().flex_1().min_w_0().pl(px(22.)).child(self.header_cell("名前", SortKey::Name, cx)))
            .child(div().flex_none().w(px(theme::DATE_WIDTH)).child(self.header_cell("更新日時", SortKey::Modified, cx)))
            .child(div().flex_none().w(px(theme::SIZE_WIDTH)).child(self.header_cell("サイズ", SortKey::Size, cx)))
            .child(div().flex_none().w(px(theme::KIND_WIDTH)).child(self.header_cell("種類", SortKey::Kind, cx)));
        let body = match &self.state.error {
            Some(message) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::SUB_TEXT))
                .child(message.clone()),
            None => div().flex_1().min_h_0().child(
                uniform_list("rows", self.state.entries.len(), cx.processor(Self::rows))
                    .track_scroll(self.scroll.clone())
                    .size_full(),
            ),
        };
        let status = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(theme::STATUS_HEIGHT))
            .px_3()
            .bg(rgb(theme::BAR))
            .border_t_1()
            .border_color(rgb(theme::LINE))
            .text_color(rgb(theme::SUB_TEXT))
            .child(match self.state.loading {
                true => format!("読み込み中… {} 項目", self.state.entries.len()),
                false => format!("{} 項目", self.state.entries.len()),
            })
            .child(self.state.path.display().to_string());
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .text_size(px(theme::TEXT_SIZE))
            .font_family(theme::FONT)
            .child(toolbar)
            .child(header)
            .child(body)
            .child(status)
    }
}
