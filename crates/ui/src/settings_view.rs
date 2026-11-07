//! 設定のウィンドウ。左で分類を選び、右に白い枠の行で並べる。項目を足す時は、分類の中の `section` へ行を足す。

use crate::open_store;
use crate::parts::{radio, slider_track, switch};
use crate::text_input::{InputEvent, TextInput};
use crate::theme;
use filer_core::{
    AppSettings, MAX_DRAG_BAR_FOLDERS, MAX_FLASH_COUNT, MAX_FLASH_HOLD_MS, MAX_FLASH_MS, MAX_SPRING_MS, MIN_SPRING_MS, NewTabFolder, ScrollbarMode, snap_flash_ms, snap_spring_ms,
};
use filer_store::Store;
use gpui::{
    AnyElement, Context, CursorStyle, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, Stateful, Window, div, prelude::*,
    px, rgb,
};

/// 全ウィンドウで共有する設定。
pub struct SettingsModel {
    pub value: AppSettings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Delay {
    OpenFirst,
    OpenNext,
    List,
    Icons,
    Column,
    Flash,
    FlashCount,
    FlashHold,
    DragFolders,
}

impl Delay {
    fn range(self) -> (u64, u64) {
        match self {
            Delay::Flash => (0, MAX_FLASH_MS),
            Delay::FlashCount => (1, MAX_FLASH_COUNT),
            Delay::FlashHold => (0, MAX_FLASH_HOLD_MS),
            Delay::DragFolders => (0, MAX_DRAG_BAR_FOLDERS),
            _ => (MIN_SPRING_MS, MAX_SPRING_MS),
        }
    }

    fn snap(self, value: f32) -> u64 {
        let (low, high) = self.range();
        match self {
            Delay::Flash => snap_flash_ms(value),
            Delay::FlashHold => (((value / 10.).round() * 10.).max(0.) as u64).clamp(low, high),
            Delay::FlashCount | Delay::DragFolders => (value.round().max(0.) as u64).clamp(low, high),
            _ => snap_spring_ms(value),
        }
    }

    fn label(self, ms: u64) -> String {
        match self {
            Delay::Flash if ms == 0 => "点滅しない".to_string(),
            Delay::Flash | Delay::FlashHold => format!("{ms} ミリ秒"),
            Delay::FlashCount => format!("{ms} 回"),
            Delay::DragFolders => format!("{ms} 個"),
            _ => format!("{:.1} 秒", ms as f32 / 1000.),
        }
    }
}

#[derive(Clone, Copy)]
struct SliderDrag {
    delay: Delay,
    start_x: f32,
    start_ms: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Tabs,
    View,
    Drag,
}

impl Page {
    const ALL: [Page; 3] = [Page::Tabs, Page::View, Page::Drag];

    fn label(self) -> &'static str {
        match self {
            Page::Tabs => "タブ",
            Page::View => "表示",
            Page::Drag => "ドラッグ",
        }
    }
}

pub struct SettingsView {
    model: Entity<SettingsModel>,
    store: Option<Store>,
    drag: Option<SliderDrag>,
    new_tab_path: Entity<TextInput>,
    page: Page,
}

fn delay_field(settings: &mut AppSettings, delay: Delay) -> &mut u64 {
    match delay {
        Delay::OpenFirst => &mut settings.spring_first_ms,
        Delay::OpenNext => &mut settings.spring_next_ms,
        Delay::List => &mut settings.spring_list_ms,
        Delay::Icons => &mut settings.spring_icons_ms,
        Delay::Column => &mut settings.spring_column_ms,
        Delay::Flash => &mut settings.flash_interval_ms,
        Delay::FlashCount => &mut settings.flash_count,
        Delay::FlashHold => &mut settings.flash_hold_ms,
        Delay::DragFolders => &mut settings.drag_bar_folders,
    }
}

/// 設定の1行。下の行との間に線を引くかは `section` が決める。
fn row(id: &'static str) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_3()
        .min_h(px(theme::SETTINGS_ROW_HEIGHT))
        .px(px(theme::SETTINGS_ROW_SIDE))
        .py_1()
}

/// 見出しと、白い枠に入れた行のまとまり。
fn section(heading: &'static str, rows: Vec<AnyElement>) -> impl IntoElement {
    let last = rows.len().saturating_sub(1);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_1p5()
        .child(div().px(px(theme::SETTINGS_ROW_SIDE)).text_size(px(theme::SMALL_TEXT_SIZE)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(rgb(theme::SUB_TEXT)).child(heading))
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(theme::SETTINGS_SECTION_RADIUS))
                .bg(rgb(theme::BACKGROUND))
                .border_1()
                .border_color(rgb(theme::LINE))
                .children(rows.into_iter().enumerate().map(|(index, line)| {
                    div().when(index != last, |item| item.border_b_1().border_color(rgb(theme::LINE))).child(line)
                })),
        )
}

impl SettingsView {
    pub fn new(model: Entity<SettingsModel>, cx: &mut Context<Self>) -> Self {
        cx.observe(&model, |_, _, cx| cx.notify()).detach();
        let path = model.read(cx).value.new_tab_path.clone();
        let length = path.len();
        let new_tab_path = cx.new(|cx| TextInput::new(path, length..length, None, cx).plain());
        cx.subscribe(&new_tab_path, |this, input, event, cx| {
            if matches!(event, InputEvent::Changed) {
                let path = input.read(cx).text();
                this.change(cx, |settings| settings.new_tab_path = path);
            }
        })
        .detach();
        Self { model, store: open_store(), drag: None, new_tab_path, page: Page::Tabs }
    }

    fn change(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut AppSettings)) {
        self.model.update(cx, |model, cx| {
            edit(&mut model.value);
            cx.notify();
        });
        self.save(cx);
    }

    fn save(&self, cx: &Context<Self>) {
        if let Some(store) = &self.store
            && let Err(error) = store.save_app_settings(&self.model.read(cx).value)
        {
            eprintln!("設定を保存できませんでした: {error}");
        }
    }

    /// オンオフの行。行のどこを押しても切り替わる。
    fn toggle(&self, id: &'static str, label: &'static str, on: bool, flip: fn(&mut AppSettings), cx: &mut Context<Self>) -> AnyElement {
        row(id)
            .cursor(CursorStyle::PointingHand)
            .child(div().flex_1().min_w_0().child(label))
            .child(switch(on))
            .on_click(cx.listener(move |this, _, _, cx| this.change(cx, flip)))
            .into_any_element()
    }

    /// 複数から1つを選ぶ行。
    fn option(&self, id: &'static str, label: &'static str, selected: bool, pick: fn(&mut AppSettings), cx: &mut Context<Self>) -> AnyElement {
        row(id)
            .cursor(CursorStyle::PointingHand)
            .child(radio(selected))
            .child(div().flex_1().min_w_0().child(label))
            .on_click(cx.listener(move |this, _, _, cx| this.change(cx, pick)))
            .into_any_element()
    }

    fn slider(&self, id: &'static str, label: &'static str, delay: Delay, value: u64, cx: &mut Context<Self>) -> AnyElement {
        let (low, high) = delay.range();
        let ratio = (value - low) as f32 / (high - low) as f32;
        row(id)
            .child(div().flex_1().min_w_0().child(label))
            .child(slider_track(id, theme::SETTINGS_SLIDER_WIDTH, theme::SLIDER_KNOB + 4., ratio).on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    let start_x = event.position.x.into();
                    this.drag = Some(SliderDrag { delay, start_x, start_ms: value as f32 });
                }),
            ))
            .child(div().flex_none().w(px(theme::SETTINGS_VALUE_WIDTH)).text_right().text_color(rgb(theme::SUB_TEXT)).child(delay.label(value)))
            .into_any_element()
    }

    fn drag_slider(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            self.save(cx);
            return;
        }
        let moved = f32::from(event.position.x) - drag.start_x;
        let (low, high) = drag.delay.range();
        let per_pixel = (high - low) as f32 / (theme::SETTINGS_SLIDER_WIDTH - theme::SLIDER_KNOB);
        let value = drag.delay.snap(drag.start_ms + moved * per_pixel);
        self.model.update(cx, |model, cx| {
            *delay_field(&mut model.value, drag.delay) = value;
            cx.notify();
        });
    }

    fn nav(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let items = Page::ALL.map(|page| {
            let active = page == self.page;
            div()
                .id(page.label())
                .flex()
                .items_center()
                .h(px(theme::SETTINGS_NAV_ROW))
                .px_2()
                .rounded(px(theme::CONTROL_CELL_RADIUS))
                .cursor(CursorStyle::PointingHand)
                .when(active, |item| item.bg(rgb(theme::SELECTED)).text_color(rgb(theme::SELECTED_TEXT)))
                .when(!active, |item| item.hover(|style| style.bg(rgb(theme::HOVER))))
                .child(page.label())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page = page;
                    cx.notify();
                }))
        });
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_0p5()
            .w(px(theme::SETTINGS_NAV_WIDTH))
            .h_full()
            .p_2()
            .bg(rgb(theme::SIDEBAR))
            .border_r_1()
            .border_color(rgb(theme::LINE))
            .children(items)
    }

    fn tabs_page(&self, settings: &AppSettings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let custom = settings.new_tab_folder == NewTabFolder::Custom;
        let mut places = vec![
            self.option("new-tab-home", "ホーム", settings.new_tab_folder == NewTabFolder::Home, |settings| settings.new_tab_folder = NewTabFolder::Home, cx),
            self.option("new-tab-current", "今のタブと同じフォルダ", settings.new_tab_folder == NewTabFolder::Current, |settings| settings.new_tab_folder = NewTabFolder::Current, cx),
            self.option("new-tab-custom", "指定したフォルダ", custom, |settings| settings.new_tab_folder = NewTabFolder::Custom, cx),
        ];
        if custom {
            places.push(
                row("new-tab-path")
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .px_2()
                            .py_0p5()
                            .rounded(px(theme::SMALL_RADIUS))
                            .border_1()
                            .border_color(rgb(theme::CONTROL_LINE))
                            .child(self.new_tab_path.clone()),
                    )
                    .into_any_element(),
            );
        }
        vec![
            section("新しいタブで開く場所", places).into_any_element(),
            section(
                "動作",
                vec![
                    self.toggle("focus-new-tabs", "フォルダを新しいタブで開いたら、そのタブへ切り替える", settings.focus_new_tabs, |settings| settings.focus_new_tabs = !settings.focus_new_tabs, cx),
                    self.toggle("wheel-tabs", "タブの帯の上のホイールで切り替える", settings.wheel_switches_tabs, |settings| settings.wheel_switches_tabs = !settings.wheel_switches_tabs, cx),
                ],
            )
            .into_any_element(),
        ]
    }

    fn view_page(&self, settings: &AppSettings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let auto_bar = settings.scrollbar == ScrollbarMode::WhileScrolling;
        vec![
            section("一覧", vec![self.toggle("show-hidden", "隠し項目を表示する（Ctrl+Shift+.）", settings.show_hidden, |settings| settings.show_hidden = !settings.show_hidden, cx)]).into_any_element(),
            section(
                "スクロールバー",
                vec![
                    self.option("bar-auto", "スクロールしている間だけ出す", auto_bar, |settings| settings.scrollbar = ScrollbarMode::WhileScrolling, cx),
                    self.option("bar-always", "常に出す", !auto_bar, |settings| settings.scrollbar = ScrollbarMode::Always, cx),
                ],
            )
            .into_any_element(),
            section(
                "アイコン表示",
                vec![
                    self.toggle("hover-play", "動画と音声はマウスを乗せると再生する", settings.hover_play, |settings| settings.hover_play = !settings.hover_play, cx),
                    self.toggle("long-scroll", "長い名前はマウスを乗せると流して見せる", settings.scroll_long_names, |settings| settings.scroll_long_names = !settings.scroll_long_names, cx),
                    self.toggle("whole-cell", "選択をアイコン全体のハイライトで示す", settings.grid_selection_matches_hover, |settings| settings.grid_selection_matches_hover = !settings.grid_selection_matches_hover, cx),
                ],
            )
            .into_any_element(),
        ]
    }

    fn drag_page(&self, settings: &AppSettings, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            section(
                "開くまでの待ち時間",
                vec![
                    self.slider("delay-list", "リストのフォルダ", Delay::List, settings.spring_list_ms, cx),
                    self.slider("delay-icons", "アイコンのフォルダ", Delay::Icons, settings.spring_icons_ms, cx),
                    self.slider("delay-column", "カラムのフォルダ", Delay::Column, settings.spring_column_ms, cx),
                    self.slider("delay-first", "帯やサイドバー（最初）", Delay::OpenFirst, settings.spring_first_ms, cx),
                    self.slider("delay-next", "帯やサイドバー（2回目以降）", Delay::OpenNext, settings.spring_next_ms, cx),
                ],
            )
            .into_any_element(),
            section(
                "開く前の点滅",
                vec![
                    self.slider("flash-interval", "点滅の間隔", Delay::Flash, settings.flash_interval_ms, cx),
                    self.slider("flash-count", "点滅する回数", Delay::FlashCount, settings.flash_count, cx),
                    self.slider("flash-hold", "開く直前の青の長さ", Delay::FlashHold, settings.flash_hold_ms, cx),
                ],
            )
            .into_any_element(),
            section(
                "ドラッグ中の下の帯",
                vec![
                    self.toggle("drag-parent", "「上の階層」を出す", settings.drag_bar_parent, |settings| settings.drag_bar_parent = !settings.drag_bar_parent, cx),
                    self.toggle("drag-back", "「戻る」を出す", settings.drag_bar_back, |settings| settings.drag_bar_back = !settings.drag_bar_back, cx),
                    self.slider("drag-folders", "並べる親フォルダの数", Delay::DragFolders, settings.drag_bar_folders, cx),
                ],
            )
            .into_any_element(),
        ]
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.model.read(cx).value.clone();
        let sections = match self.page {
            Page::Tabs => self.tabs_page(&settings, cx),
            Page::View => self.view_page(&settings, cx),
            Page::Drag => self.drag_page(&settings, cx),
        };
        div()
            .id("settings")
            .flex()
            .size_full()
            .bg(rgb(theme::SETTINGS_BACK))
            .text_color(rgb(theme::TEXT))
            .text_size(px(theme::TEXT_SIZE))
            .font_family(theme::FONT)
            .on_mouse_move(cx.listener(Self::drag_slider))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag.take().is_some() {
                        this.save(cx);
                    }
                }),
            )
            .child(self.nav(cx))
            .child(
                div()
                    .id(self.page.label())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap_4()
                    .p(px(theme::SETTINGS_PAGE_PADDING))
                    .overflow_y_scroll()
                    .child(div().flex_none().px(px(theme::SETTINGS_ROW_SIDE)).text_size(px(theme::SETTINGS_TITLE_SIZE)).font_weight(gpui::FontWeight::SEMIBOLD).child(self.page.label()))
                    .children(sections),
            )
    }
}
