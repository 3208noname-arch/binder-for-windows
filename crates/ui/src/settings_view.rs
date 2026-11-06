//! 設定のウィンドウ。項目が増えても並べられるよう、本体とは別に開く。

use crate::open_store;
use crate::theme;
use filer_core::{AppSettings, DragHover, LongName, MAX_SPRING_MS, MIN_SPRING_MS, ScrollbarMode, snap_spring_ms};
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
    Popup,
    Column,
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
        Delay::Popup => &mut settings.spring_popup_ms,
        Delay::Column => &mut settings.spring_column_ms,
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
        let range = (MAX_SPRING_MS - MIN_SPRING_MS) as f32;
        let ratio = (value - MIN_SPRING_MS) as f32 / range;
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
            .child(div().w(px(56.)).text_color(rgb(theme::SUB_TEXT)).child(format!("{:.1} 秒", value as f32 / 1000.)))
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
        let per_pixel = (MAX_SPRING_MS - MIN_SPRING_MS) as f32 / (theme::SETTINGS_SLIDER_WIDTH - 14.);
        let value = snap_spring_ms(drag.start_ms + moved * per_pixel);
        self.model.update(cx, |model, cx| {
            *delay_field(&mut model.value, drag.delay) = value;
            cx.notify();
        });
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = self.model.read(cx).value.clone();
        let open = settings.drag_hover == DragHover::Open;
        let fade = settings.long_name == LongName::Fade;
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
            .child(Self::heading("ドラッグ中にフォルダの上で止まった時（リスト、アイコン）"))
            .child(Self::choice("hover-open", "フォルダを開く", open).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.drag_hover = DragHover::Open);
            })))
            .child(Self::choice("hover-popup", "中身を小窓で見せる", !open).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.drag_hover = DragHover::Popup);
            })))
            .child(Self::heading("ドラッグ中に開くまでの待ち時間"))
            .child(self.slider("delay-first", "フォルダを開く（最初）", Delay::OpenFirst, settings.spring_first_ms, cx))
            .child(self.slider("delay-next", "フォルダを開く（2回目以降）", Delay::OpenNext, settings.spring_next_ms, cx))
            .child(self.slider("delay-popup", "小窓を出す", Delay::Popup, settings.spring_popup_ms, cx))
            .child(self.slider("delay-column", "カラムの列を開く", Delay::Column, settings.spring_column_ms, cx))
            .child(Self::heading("スクロールバー"))
            .child(Self::choice("bar-auto", "スクロールしている間だけ出す", auto_bar).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.scrollbar = ScrollbarMode::WhileScrolling);
            })))
            .child(Self::choice("bar-always", "常に出す", !auto_bar).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.scrollbar = ScrollbarMode::Always);
            })))
            .child(Self::heading("アイコン表示"))
            .child(Self::choice("long-fade", "長い名前は最後を薄く消す", fade).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.long_name = LongName::Fade);
            })))
            .child(Self::choice("long-scroll", "長い名前はマウスを乗せると流して見せる", !fade).on_click(cx.listener(|this, _, _, cx| {
                this.change(cx, |settings| settings.long_name = LongName::Scroll);
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
