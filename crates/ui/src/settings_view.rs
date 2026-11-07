//! 設定のウィンドウ。項目が増えても並べられるよう、本体とは別に開く。

use crate::open_store;
use crate::theme;
use filer_core::{AppSettings, MAX_FLASH_MS, MAX_SPRING_MS, MIN_SPRING_MS, ScrollbarMode, snap_flash_ms, snap_spring_ms};
use filer_store::Store;
use gpui::{
    AnyElement, Context, CursorStyle, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, Stateful, Window, div, prelude::*,
    px, rgb, rgba,
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
}

impl Delay {
    fn range(self) -> (u64, u64) {
        if self == Delay::Flash { (0, MAX_FLASH_MS) } else { (MIN_SPRING_MS, MAX_SPRING_MS) }
    }

    fn snap(self, ms: f32) -> u64 {
        if self == Delay::Flash { snap_flash_ms(ms) } else { snap_spring_ms(ms) }
    }

    fn label(self, ms: u64) -> String {
        match self {
            Delay::Flash if ms == 0 => "点滅しない".to_string(),
            Delay::Flash => format!("{ms} ミリ秒"),
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

pub struct SettingsView {
    model: Entity<SettingsModel>,
    store: Option<Store>,
    drag: Option<SliderDrag>,
}

fn delay_field(settings: &mut AppSettings, delay: Delay) -> &mut u64 {
    match delay {
        Delay::OpenFirst => &mut settings.spring_first_ms,
        Delay::OpenNext => &mut settings.spring_next_ms,
        Delay::List => &mut settings.spring_list_ms,
        Delay::Icons => &mut settings.spring_icons_ms,
        Delay::Column => &mut settings.spring_column_ms,
        Delay::Flash => &mut settings.move_flash_ms,
    }
}

impl SettingsView {
    pub fn new(model: Entity<SettingsModel>, cx: &mut Context<Self>) -> Self {
        cx.observe(&model, |_, _, cx| cx.notify()).detach();
        Self { model, store: open_store(), drag: None }
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

    fn heading(label: &'static str) -> impl IntoElement {
        div()
            .pt_4()
            .pb_1()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .text_color(rgb(theme::SUB_TEXT))
            .border_b_1()
            .border_color(rgb(theme::LINE))
            .child(label)
    }

    fn choice(id: &'static str, label: &'static str, checked: bool) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_2()
            .py_1()
            .cursor(CursorStyle::PointingHand)
            .child(
                div()
                    .size(px(14.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(theme::SUB_TEXT))
                    .when(checked, |mark| mark.bg(rgb(theme::SELECTED))),
            )
            .child(label)
    }

    fn slider(&self, id: &'static str, label: &'static str, delay: Delay, value: u64, cx: &mut Context<Self>) -> AnyElement {
        let (low, high) = delay.range();
        let ratio = (value - low) as f32 / (high - low) as f32;
        div()
            .flex()
            .items_center()
            .gap_3()
            .py_1()
            .child(div().w(px(theme::SETTINGS_LABEL_WIDTH)).child(label))
            .child(
                div()
                    .id(id)
                    .relative()
                    .flex_none()
                    .w(px(theme::SETTINGS_SLIDER_WIDTH))
                    .h(px(20.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .child(div().absolute().top(px(9.)).left_0().right_0().h(px(3.)).rounded_full().bg(rgba(theme::SCROLL_THUMB)))
                    .child(
                        div()
                            .absolute()
                            .top(px(3.))
                            .left(px(ratio * (theme::SETTINGS_SLIDER_WIDTH - 14.)))
                            .size(px(14.))
                            .rounded_full()
                            .bg(rgba(theme::ICON_SLIDER_KNOB)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, _| {
                            let start_x = event.position.x.into();
                            this.drag = Some(SliderDrag { delay, start_x, start_ms: value as f32 });
                        }),
                    ),
            )
            .child(div().w(px(80.)).text_color(rgb(theme::SUB_TEXT)).child(delay.label(value)))
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
        let per_pixel = (high - low) as f32 / (theme::SETTINGS_SLIDER_WIDTH - 14.);
        let value = drag.delay.snap(drag.start_ms + moved * per_pixel);
        self.model.update(cx, |model, cx| {
            *delay_field(&mut model.value, drag.delay) = value;
            cx.notify();
        });
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.model.read(cx).value.clone();
        let auto_bar = settings.scrollbar == ScrollbarMode::WhileScrolling;
        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .bg(rgb(theme::BACKGROUND))
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
            .child(Self::heading("ドラッグ中に開くまでの待ち時間"))
            .child(self.slider("delay-list", "リストのフォルダ", Delay::List, settings.spring_list_ms, cx))
            .child(self.slider("delay-icons", "アイコンのフォルダ", Delay::Icons, settings.spring_icons_ms, cx))
            .child(self.slider("delay-column", "カラムのフォルダ", Delay::Column, settings.spring_column_ms, cx))
            .child(self.slider("delay-first", "帯やサイドバー（最初）", Delay::OpenFirst, settings.spring_first_ms, cx))
            .child(self.slider("delay-next", "帯やサイドバー（2回目以降）", Delay::OpenNext, settings.spring_next_ms, cx))
            .child(Self::heading("タブ"))
            .child(Self::choice("focus-new-tabs", "フォルダを新しいタブで開いたら、そのタブへ切り替える", settings.focus_new_tabs).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.focus_new_tabs = !settings.focus_new_tabs);
            })))
            .child(Self::choice("wheel-tabs", "タブの帯の上のホイールで切り替える", settings.wheel_switches_tabs).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.wheel_switches_tabs = !settings.wheel_switches_tabs);
            })))
            .child(Self::heading("ファイルの移動"))
            .child(self.slider("flash-interval", "移動の直前の点滅の間隔", Delay::Flash, settings.move_flash_ms, cx))
            .child(Self::heading("一覧"))
            .child(Self::choice("show-hidden", "隠し項目を表示する（Ctrl+Shift+.）", settings.show_hidden).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.show_hidden = !settings.show_hidden);
            })))
            .child(Self::heading("スクロールバー"))
            .child(Self::choice("bar-auto", "スクロールしている間だけ出す", auto_bar).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.scrollbar = ScrollbarMode::WhileScrolling);
            })))
            .child(Self::choice("bar-always", "常に出す", !auto_bar).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.scrollbar = ScrollbarMode::Always);
            })))
            .child(Self::heading("アイコン表示"))
            .child(Self::choice("long-scroll", "長い名前はマウスを乗せると流して見せる", settings.scroll_long_names).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.scroll_long_names = !settings.scroll_long_names);
            })))
            .child(
                Self::choice("whole-cell", "選択をアイコン全体のハイライトで示す", settings.grid_selection_matches_hover).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.change(cx, |settings| {
                            settings.grid_selection_matches_hover = !settings.grid_selection_matches_hover;
                        });
                    }),
                ),
            )
    }
}
