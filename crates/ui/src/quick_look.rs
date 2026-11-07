//! スペースキーのクイックルック。Finderと同じ独立したウィンドウで、キーの操作は一覧に残す。

use crate::browser::{Browser, window_handle};
use crate::icon_cache::{IconCache, LARGE};
use crate::media_view::{MediaSurface, picture_image};
use crate::parts::{Icon, icon};
use crate::theme;
use filer_core::{Entry, PreviewKind, Tree, build_tree, fit_window, format_size, kind_of, preview_kind, visible_lines};
use filer_shell::{Pdf, compare_names, decode_image, fit_window_client, format_time, read_text_head, read_zip, set_window_owner, walk_folder};
use gpui::{
    AnyWindowHandle, App, Bounds, Context, Entity, FocusHandle, Focusable, KeyDownEvent, RenderImage, ScrollWheelEvent, SharedString, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, div, img, prelude::*, px, rgb, size, uniform_list,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// テキストを読む上限。先頭だけ見せれば、種類を確かめる用は足りる。
const TEXT_LIMIT: usize = 256 * 1024;
const LINE_LIMIT: usize = 2000;
const LINE_HEIGHT: f32 = 20.;
/// 画像を読む大きさの上限。ウィンドウを広げても粗くならない大きさにする。
const PICTURE_LIMIT: u32 = 2560;
/// 矢印キーを押し続けた間は読み始めない。
const SETTLE: Duration = Duration::from_millis(80);
const AUDIO_BAR: f32 = 48.;
/// フォルダの配下を読む時間の上限。遅いNASで待たせ続けない。
const WALK_BUDGET: Duration = Duration::from_secs(3);
const TREE_ROW: f32 = 24.;
const TREE_INDENT: f32 = 18.;
const GLYPH_FONT: &str = "Segoe MDL2 Assets";
const GLYPH_CLOSED: &str = "\u{E76C}";
const GLYPH_OPEN: &str = "\u{E70D}";
const GLYPH_FOLDER: &str = "\u{E8B7}";
const GLYPH_FILE: &str = "\u{E7C3}";
/// 絵の周りの余白
const PICTURE_PADDING: f32 = 8.;
/// 動画の下の操作の高さ
const MEDIA_CONTROLS: f32 = 32.;
/// PDFのページの長辺の上限。文字が読める細かさにする。
const PDF_LIMIT: u32 = 1800;
/// ページの下の操作の高さ
const PAGE_BAR: f32 = 32.;
/// 1ページ送るホイールの回転量
const WHEEL_PAGE: f32 = 60.;

enum Content {
    /// 読んでいる途中
    Loading,
    Picture(Arc<RenderImage>),
    Text { lines: Arc<Vec<SharedString>>, cut: bool },
    Media { surface: Entity<MediaSurface>, audio: bool },
    /// PDF。ページの絵は開くたびに作る
    Pdf { pdf: Pdf, page: u32, image: Option<Arc<RenderImage>> },
    /// Zipの目次やフォルダの中身
    Tree { tree: Arc<Tree>, closed: HashSet<usize>, visible: Rc<Vec<usize>> },
    /// 中身を見せられない項目
    Info,
}

pub struct QuickLook {
    browser: WeakEntity<Browser>,
    owner: AnyWindowHandle,
    focus: FocusHandle,
    icons: Entity<IconCache>,
    item: (PathBuf, Entry),
    content: Content,
    load: Option<Task<()>>,
    watch: Option<Subscription>,
    /// ホイールの、まだページ送りに使っていない回転量
    wheel: f32,
    hwnd: isize,
    /// 今のウィンドウの大きさを決めた、中身の縦横と操作の高さ
    fitted: Option<(f32, f32, f32)>,
}

/// 読んだテキストを行に分ける。長い行は切り、タブは見える幅にする。
fn text_lines(text: &str) -> Vec<SharedString> {
    text.lines().map(|line| line.chars().take(LINE_LIMIT).collect::<String>().replace('\t', "    ").into()).collect()
}

impl QuickLook {
    /// ウィンドウを開く。一覧からフォーカスを奪わない。
    pub fn open(browser: &mut Browser, folder: PathBuf, entry: Entry, owner: &Window, cx: &mut Context<Browser>) -> Option<WindowHandle<Self>> {
        let parent = owner.bounds();
        let wanted = browser.quick_look_bounds.unwrap_or_else(|| {
            let window_size = size(px(theme::QUICK_LOOK_WIDTH), px(theme::QUICK_LOOK_HEIGHT));
            Bounds::new(parent.center() - gpui::point(window_size.width / 2., window_size.height / 2.), window_size)
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(wanted)),
            titlebar: Some(TitlebarOptions { title: Some(entry.name.clone().into()), ..Default::default() }),
            focus: false,
            kind: WindowKind::Normal,
            ..Default::default()
        };
        let weak = cx.entity().downgrade();
        let owner_handle = owner.window_handle();
        let owner_hwnd = browser.hwnd;
        let icons = browser.icons.clone();
        let show_hidden = browser.settings.show_hidden;
        cx.open_window(options, move |window, cx| {
            cx.new(|cx| {
                let hwnd = window_handle(window);
                cx.defer(move |_| set_window_owner(hwnd, owner_hwnd));
                let close_weak = weak.clone();
                window.on_window_should_close(cx, move |window, cx| {
                    let bounds = window.bounds();
                    let _ = close_weak.update(cx, |browser, cx| browser.quick_look_closed(bounds, cx));
                    true
                });
                cx.observe(&icons, |_, _, cx| cx.notify()).detach();
                let mut view = Self {
                    browser: weak,
                    owner: owner_handle,
                    focus: cx.focus_handle(),
                    icons,
                    item: (folder.clone(), entry.clone()),
                    content: Content::Loading,
                    load: None,
                    watch: None,
                    wheel: 0.,
                    hwnd,
                    fitted: None,
                };
                view.show(folder, entry, show_hidden, window, cx);
                // このウィンドウが押された時にキーを受け取れるようにする。一覧のフォーカスには影響しない
                window.focus(&view.focus);
                view
            })
        })
        .ok()
    }

    /// 見せる項目を変える。前の項目の再生と読み込みは、ここで捨てる。
    /// 開く時は一覧の更新の途中なので、一覧の設定は呼び出し側から受け取る。
    pub fn show(&mut self, folder: PathBuf, entry: Entry, show_hidden: bool, window: &mut Window, cx: &mut Context<Self>) {
        let path = folder.join(entry.os_name());
        let kind = preview_kind(&entry.name, entry.is_dir);
        window.set_window_title(&entry.name);
        self.item = (folder, entry);
        self.load = None;
        match &mut self.content {
            Content::Picture(image) => window.drop_image(image.clone()).ok(),
            Content::Pdf { image, .. } => image.take().and_then(|image| window.drop_image(image).ok()),
            _ => None,
        };
        self.content = match kind {
            PreviewKind::Video | PreviewKind::Audio => {
                let surface = cx.new(|cx| MediaSurface::new(path, kind, false, false, None, window, cx));
                // 再生できないと分かった時に描き直す
                self.watch = Some(cx.observe(&surface, |_, _, cx| cx.notify()));
                Content::Media { surface, audio: kind == PreviewKind::Audio }
            }
            PreviewKind::Image => {
                self.load = Some(Self::load_picture(path, window, cx));
                Content::Loading
            }
            PreviewKind::Text => {
                self.load = Some(Self::load_text(path, window, cx));
                Content::Loading
            }
            PreviewKind::Pdf => {
                self.load = Some(Self::load_pdf(path, window, cx));
                Content::Loading
            }
            PreviewKind::Folder | PreviewKind::Archive => {
                self.load = Some(Self::load_tree(path, kind == PreviewKind::Archive, show_hidden, window, cx));
                Content::Loading
            }
            PreviewKind::Other => Content::Info,
        };
        cx.notify();
    }

    fn load_picture(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let picture = cx.background_executor().spawn(async move { decode_image(&path, PICTURE_LIMIT, PICTURE_LIMIT) }).await;
            let _ = this.update(cx, |this, cx| {
                this.content = picture.ok().and_then(picture_image).map_or(Content::Info, Content::Picture);
                cx.notify();
            });
        })
    }

    fn load_tree(path: PathBuf, archive: bool, show_hidden: bool, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let tree = cx
                .background_executor()
                .spawn(async move {
                    let (items, cut) = if archive { read_zip(&path).ok()? } else { walk_folder(&path, show_hidden, WALK_BUDGET) };
                    Some(build_tree(items, cut, compare_names))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.content = tree.map_or(Content::Info, |tree| {
                    let closed = tree.initially_closed();
                    let visible = Rc::new(visible_lines(&tree, &closed));
                    Content::Tree { tree: Arc::new(tree), closed, visible }
                });
                cx.notify();
            });
        })
    }

    /// フォルダの行を開閉する。
    fn toggle_line(&mut self, line: usize, cx: &mut Context<Self>) {
        if let Content::Tree { tree, closed, visible } = &mut self.content {
            if !closed.remove(&line) {
                closed.insert(line);
            }
            *visible = Rc::new(visible_lines(tree, closed));
            cx.notify();
        }
    }

    fn tree_row(tree: &Tree, line_index: usize, open: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let line = &tree.lines[line_index];
        let glyph = |text: &'static str, color: u32| div().flex_none().w(px(16.)).font_family(GLYPH_FONT).text_size(px(12.)).text_color(rgb(color)).child(text);
        let arrow = if !line.has_children {
            ""
        } else if open {
            GLYPH_OPEN
        } else {
            GLYPH_CLOSED
        };
        div()
            .id(line_index)
            .flex()
            .items_center()
            .gap_1()
            .w_full()
            .h(px(TREE_ROW))
            .pl(px(12. + line.depth as f32 * TREE_INDENT))
            .pr_3()
            .hover(|row| row.bg(rgb(theme::HOVER)))
            .child(glyph(arrow, theme::SUB_TEXT))
            .child(glyph(if line.is_dir { GLYPH_FOLDER } else { GLYPH_FILE }, if line.is_dir { theme::FOLDER } else { theme::SUB_TEXT }))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(line.name.clone()))
            .child(div().flex_none().text_size(px(theme::SMALL_TEXT_SIZE)).text_color(rgb(theme::SUB_TEXT)).child(format_size(line.size)))
            .when(line.has_children, |row| row.on_click(cx.listener(move |this, _, _, cx| this.toggle_line(line_index, cx))))
    }

    fn tree_view(&self, tree: Arc<Tree>, closed: &HashSet<usize>, visible: Rc<Vec<usize>>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let archive = preview_kind(&self.item.1.name, self.item.1.is_dir) == PreviewKind::Archive;
        let mut summary = format!("{} 個のファイル、{} 個のフォルダ、{}", tree.files, tree.folders, format_size(tree.total));
        if archive {
            summary.push_str("（展開後）");
        }
        if tree.cut {
            summary.push_str("。多いので途中までを表示しています");
        }
        let body = if visible.is_empty() {
            div().flex().flex_1().items_center().justify_center().text_color(rgb(theme::SUB_TEXT)).child(if archive { "空のZipです" } else { "空のフォルダです" }).into_any_element()
        } else {
            let closed = closed.clone();
            let count = visible.len();
            let tree = tree.clone();
            uniform_list(
                "quick-look-tree",
                count,
                cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                    range.map(|index| Self::tree_row(&tree, visible[index], !closed.contains(&visible[index]), cx)).collect()
                }),
            )
            .flex_1()
            .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(body)
            .child(div().flex_none().px_3().py_1().text_size(px(theme::SMALL_TEXT_SIZE)).text_color(rgb(theme::SUB_TEXT)).child(summary))
    }

    fn load_pdf(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let opened = cx
                .background_executor()
                .spawn(async move {
                    let pdf = Pdf::open(&path).ok().filter(|pdf| pdf.pages > 0)?;
                    let picture = pdf.render(0, PDF_LIMIT, PDF_LIMIT).ok()?;
                    Some((pdf, picture))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.content = opened.map_or(Content::Info, |(pdf, picture)| Content::Pdf { pdf, page: 0, image: picture_image(picture) });
                cx.notify();
            });
        })
    }

    /// PDFのページを送る。前のページの絵は、次の絵が届くまで残す。
    pub fn turn_page(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Content::Pdf { pdf, page, .. } = &mut self.content else { return };
        let target = page.saturating_add_signed(delta).min(pdf.pages - 1);
        if target == *page {
            return;
        }
        *page = target;
        let pdf = pdf.clone();
        cx.notify();
        self.load = Some(cx.spawn_in(window, async move |this, cx| {
            let picture = cx.background_executor().spawn(async move { pdf.render(target, PDF_LIMIT, PDF_LIMIT) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if let Content::Pdf { image, .. } = &mut this.content {
                    if let Some(old) = std::mem::replace(image, picture.ok().and_then(picture_image)) {
                        window.drop_image(old).ok();
                    }
                    cx.notify();
                }
            });
        }));
    }

    fn scroll_pages(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.wheel += f32::from(event.delta.pixel_delta(px(LINE_HEIGHT)).y);
        if self.wheel.abs() >= WHEEL_PAGE {
            let delta = if self.wheel > 0. { -1 } else { 1 };
            self.wheel = 0.;
            self.turn_page(delta, window, cx);
        }
    }

    fn pdf_view(&self, image: Option<Arc<RenderImage>>, page: u32, pages: u32, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let turn = |id: &'static str, kind: Icon, delta: i32, enabled: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .size(px(24.))
                .rounded(px(theme::SMALL_RADIUS))
                .when(enabled, |button| button.hover(|button| button.bg(rgb(theme::HOVER))))
                .child(icon(kind, 12., if enabled { theme::TEXT } else { theme::DISABLED_TEXT }))
                .on_click(cx.listener(move |this, _, window, cx| this.turn_page(delta, window, cx)))
        };
        div()
            .id("quick-look-pdf")
            .flex()
            .flex_col()
            .size_full()
            .on_scroll_wheel(cx.listener(Self::scroll_pages))
            .child(div().flex_1().min_h_0().p_2().bg(rgb(theme::STRIPE)).children(image.map(|image| img(image).size_full())))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .h(px(PAGE_BAR))
                    .text_size(px(theme::SMALL_TEXT_SIZE))
                    .child(turn("pdf-previous", Icon::Back, -1, page > 0, cx))
                    .child(format!("{} / {}", page + 1, pages))
                    .child(turn("pdf-next", Icon::Forward, 1, page + 1 < pages, cx)),
            )
    }

    fn load_text(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SETTLE).await;
            let read = cx
                .background_executor()
                .spawn(async move {
                    let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                    read_text_head(&path, TEXT_LIMIT).map(|text| (text_lines(&text), size > TEXT_LIMIT as u64))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.content = read.map_or(Content::Info, |(lines, cut)| Content::Text { lines: Arc::new(lines), cut });
                cx.notify();
            });
        })
    }

    /// 今の中身の縦横（論理ピクセルの比）と、絵の外に要る高さ。形の無い中身は決まった大きさにする。
    fn content_shape(&self, scale: f32, cx: &App) -> (f32, f32, f32) {
        let picture = |image: &Arc<RenderImage>, extra: f32| {
            let size = image.size(0);
            (i32::from(size.width) as f32 / scale, i32::from(size.height) as f32 / scale, extra)
        };
        match &self.content {
            Content::Picture(image) => picture(image, PICTURE_PADDING * 2.),
            Content::Pdf { image: Some(image), .. } => picture(image, PICTURE_PADDING * 2. + PAGE_BAR),
            Content::Media { surface, audio: false } => match surface.read(cx).aspect() {
                Some(aspect) => (1., aspect, MEDIA_CONTROLS),
                None => (0., 0., 0.),
            },
            _ => (0., 0., 0.),
        }
    }

    /// 中身の形が変わっていたら、ウィンドウを合わせる。
    fn fit_to_content(&mut self, window: &Window, cx: &mut Context<Self>) {
        let scale = window.scale_factor();
        let shape = self.content_shape(scale, cx);
        if self.fitted == Some(shape) || (shape.0 <= 0. && matches!(self.content, Content::Loading)) {
            return;
        }
        self.fitted = Some(shape);
        let (width, height) = if shape.0 > 0. {
            fit_window(shape.0, shape.1, shape.2, theme::QUICK_LOOK_MAX_WIDTH, theme::QUICK_LOOK_MAX_HEIGHT)
        } else {
            (theme::QUICK_LOOK_WIDTH, theme::QUICK_LOOK_HEIGHT)
        };
        let (hwnd, width, height) = (self.hwnd, (width * scale) as i32, (height * scale) as i32);
        // メッセージが回るので、この更新を抜けてから動かす
        cx.spawn(async move |_, _| fit_window_client(hwnd, width, height)).detach();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = window.bounds();
        let _ = self.browser.update(cx, |browser, cx| browser.quick_look_closed(bounds, cx));
        window.remove_window();
    }

    /// このウィンドウを押した後も、矢印キーなどは一覧の操作として働かせる。
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.is_held && matches!(event.keystroke.key.as_str(), "space" | "escape") {
            return;
        }
        match event.keystroke.key.as_str() {
            "space" | "escape" => return self.close(window, cx),
            "pageup" => return self.turn_page(-1, window, cx),
            "pagedown" => return self.turn_page(1, window, cx),
            _ => {}
        }
        let event = event.clone();
        let (browser, owner) = (self.browser.clone(), self.owner);
        cx.defer(move |cx: &mut App| {
            let _ = owner.update(cx, |_, window, cx| {
                let _ = browser.update(cx, |browser, cx| browser.key_down(&event, window, cx));
            });
        });
    }

    fn info(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (folder, entry) = &self.item;
        let icon = self.icons.update(cx, |icons, _| icons.get(folder, entry, LARGE));
        let line = |label: &str, value: String| {
            div().flex().gap_2().text_size(px(theme::SMALL_TEXT_SIZE)).child(div().text_color(rgb(theme::SUB_TEXT)).child(label.to_string())).child(value)
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .size_full()
            .p_4()
            .children(icon.map(|icon| img(icon).size(px(theme::PREVIEW_ICON))))
            .child(div().text_center().child(entry.name.clone()))
            .child(line("種類", kind_of(entry)))
            .child(line("サイズ", format_size(entry.size)))
            .child(line("更新日時", format_time(entry.modified)))
    }

    fn text_view(lines: Arc<Vec<SharedString>>, cut: bool) -> impl IntoElement {
        let count = lines.len();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                uniform_list("quick-look-text", count, move |range, _, _| {
                    range.map(|index| div().h(px(LINE_HEIGHT)).px_3().whitespace_nowrap().overflow_hidden().child(lines[index].clone())).collect()
                })
                .flex_1(),
            )
            .when(cut, |view| view.child(div().flex_none().px_3().py_1().text_size(px(theme::SMALL_TEXT_SIZE)).text_color(rgb(theme::SUB_TEXT)).child("先頭だけを表示しています")))
    }
}

impl Focusable for QuickLook {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for QuickLook {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.fit_to_content(window, cx);
        let body = match &self.content {
            Content::Loading => div().size_full().into_any_element(),
            Content::Picture(image) => div().size_full().p_2().child(img(image.clone()).size_full()).into_any_element(),
            Content::Text { lines, cut } => Self::text_view(lines.clone(), *cut).into_any_element(),
            Content::Media { surface, audio } if *audio || surface.read(cx).failed() => {
                let surface = surface.clone();
                let info = self.info(cx);
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(div().flex_1().min_h_0().child(info))
                    .child(div().flex_none().h(px(AUDIO_BAR)).px_3().child(surface))
                    .into_any_element()
            }
            Content::Media { surface, .. } => div().size_full().child(surface.clone()).into_any_element(),
            Content::Pdf { pdf, page, image } => {
                let (image, page, pages) = (image.clone(), *page, pdf.pages);
                self.pdf_view(image, page, pages, cx).into_any_element()
            }
            Content::Tree { tree, closed, visible } => {
                let (tree, closed, visible) = (tree.clone(), closed.clone(), visible.clone());
                self.tree_view(tree, &closed, visible, cx).into_any_element()
            }
            Content::Info => self.info(cx).into_any_element(),
        };
        div()
            .size_full()
            .bg(rgb(theme::BACKGROUND))
            .text_color(rgb(theme::TEXT))
            .text_size(px(theme::TEXT_SIZE))
            .font_family(theme::FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .child(body)
    }
}

impl Browser {
    /// 選択の先頭の項目。カーソルがある項目を選び、無ければどれか1つを使う。
    fn quick_look_target(&self, cx: &App) -> Option<(PathBuf, Entry)> {
        let path = self.tab.selection.cursor().filter(|path| self.tab.selection.contains(path)).map(Path::to_path_buf).or_else(|| self.tab.selection.paths().pop())?;
        self.find_entry(&path, cx).map(|entry| (path.parent().unwrap_or(&path).to_path_buf(), entry))
    }

    /// スペースキー。開いていれば閉じる。
    pub fn toggle_quick_look(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(handle) = self.quick_look.take() {
            let closed = handle.update(cx, |_, window, _| {
                let bounds = window.bounds();
                window.remove_window();
                bounds
            });
            self.quick_look_bounds = closed.ok().or(self.quick_look_bounds);
            self.quick_look_item = None;
            cx.notify();
            return true;
        }
        let Some((folder, entry)) = self.quick_look_target(cx) else { return false };
        self.quick_look_item = Some(folder.join(entry.os_name()));
        self.quick_look = QuickLook::open(self, folder, entry, window, cx);
        cx.notify();
        true
    }

    /// 一覧にフォーカスがある時の、PDFのページ送り。
    pub fn quick_look_page(&mut self, delta: i32, cx: &mut Context<Self>) -> bool {
        let Some(handle) = self.quick_look else { return false };
        cx.defer(move |cx| {
            let _ = handle.update(cx, |view, window, cx| view.turn_page(delta, window, cx));
        });
        true
    }

    /// クイックルックが自分で閉じた。
    pub fn quick_look_closed(&mut self, bounds: Bounds<gpui::Pixels>, cx: &mut Context<Self>) {
        self.quick_look = None;
        self.quick_look_item = None;
        self.quick_look_bounds = Some(bounds);
        self.refocus = true;
        cx.notify();
    }

    /// 選択が変わっていれば、クイックルックの中身を追従させる。
    pub fn sync_quick_look(&mut self, cx: &mut Context<Self>) {
        let Some(handle) = self.quick_look else { return };
        let Some((folder, entry)) = self.quick_look_target(cx) else { return };
        let path = folder.join(entry.os_name());
        if self.quick_look_item.as_ref() == Some(&path) {
            return;
        }
        self.quick_look_item = Some(path);
        let show_hidden = self.settings.show_hidden;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |view, window, cx| view.show(folder, entry, show_hidden, window, cx));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_cut_and_tabs_widened() {
        let text = format!("a\tb\n{}", "x".repeat(LINE_LIMIT + 10));
        let lines = text_lines(&text);
        assert_eq!(lines[0].as_ref(), "a    b");
        assert_eq!(lines[1].chars().count(), LINE_LIMIT);
    }

    #[test]
    fn empty_text_has_no_lines() {
        assert!(text_lines("").is_empty());
    }
}
