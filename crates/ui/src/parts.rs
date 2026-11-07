//! 画面に共通の部品。アイコン、ボタンの枠、スイッチ、ラジオ、スライダーの見た目をここに集める。

use crate::theme;
use gpui::{AnyView, App, AssetSource, Context, Div, Render, Result, SharedString, Stateful, Svg, Window, div, prelude::*, px, rgb, rgba, svg};
use std::borrow::Cow;

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        /// 実行ファイルへ埋め込むアイコン。色は `icon` で付ける。
        #[derive(Clone, Copy)]
        pub enum Icon { $($name,)* }

        impl Icon {
            const ALL: &[Icon] = &[$(Icon::$name,)*];

            fn path(self) -> &'static str {
                match self { $(Icon::$name => concat!("icons/", $file, ".svg"),)* }
            }

            fn bytes(self) -> &'static [u8] {
                match self { $(Icon::$name => include_bytes!(concat!("../assets/icons/", $file, ".svg")),)* }
            }
        }
    };
}

icons! {
    Back => "back",
    Forward => "forward",
    Up => "up",
    ChevronDown => "chevron-down",
    ChevronUp => "chevron-up",
    ChevronRight => "chevron-right",
    Check => "check",
    Close => "close",
    Plus => "plus",
    Minus => "minus",
    Shortcut => "shortcut",
    Columns => "columns",
    List => "list",
    Grid => "grid",
    Eye => "eye",
    Search => "search",
    Gear => "gear",
    Sort => "sort",
    Group => "group",
    Play => "play",
    Pause => "pause",
    Sound => "sound",
    Muted => "muted",
}

/// `gpui` へアイコンを渡す。ファイルを読まないので、配布物は1つで済む。
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::ALL.iter().find(|icon| icon.path() == path).map(|icon| Cow::Borrowed(icon.bytes())))
    }

    fn list(&self, _: &str) -> Result<Vec<SharedString>> {
        Ok(Icon::ALL.iter().map(|icon| icon.path().into()).collect())
    }
}

pub fn icon(icon: Icon, size: f32, color: u32) -> Svg {
    svg().path(icon.path()).flex_none().size(px(size)).text_color(rgb(color))
}

struct Tip(&'static str);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_0p5()
            .rounded(px(theme::SMALL_RADIUS))
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::LINE))
            .shadow_sm()
            .font_family(theme::FONT)
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .text_color(rgb(theme::TEXT))
            .child(self.0)
    }
}

/// マウスを乗せた時に出す名前。アイコンだけのボタンに付ける。
pub fn tip(text: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tip(text)).into()
}

/// ツールバーのボタンをまとめる、白い角丸の枠。
pub fn control_group() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(theme::CONTROL_HEIGHT))
        .p(px(theme::CONTROL_INSET))
        .rounded(px(theme::CONTROL_RADIUS))
        .bg(rgb(theme::BACKGROUND))
        .border_1()
        .border_color(rgb(theme::LINE))
}

/// 枠の中の押せる区画。アイコンの色は呼ぶ側で決める。
pub fn control_cell(id: impl Into<gpui::ElementId>, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h_full()
        .rounded(px(theme::CONTROL_CELL_RADIUS))
        .when(enabled, |cell| {
            cell.cursor(gpui::CursorStyle::PointingHand).hover(|style| style.bg(rgb(theme::HOVER))).active(|style| style.bg(rgb(theme::SEGMENT)))
        })
}

/// アイコン1つの区画。
pub fn icon_cell(id: impl Into<gpui::ElementId>, kind: Icon, enabled: bool) -> Stateful<Div> {
    control_cell(id, enabled)
        .w(px(theme::CONTROL_CELL_WIDTH))
        .child(icon(kind, theme::CONTROL_ICON, if enabled { theme::TEXT } else { theme::DISABLED_TEXT }))
}

/// 枠の中の区切り線。
pub fn control_divider() -> Div {
    div().flex_none().mx(px(theme::CONTROL_INSET)).w(px(1.)).h(px(theme::NAV_DIVIDER_HEIGHT)).bg(rgb(theme::LINE))
}

/// オンオフのスイッチ。値の表示だけを持ち、押した時の処理は行の側に付ける。
pub fn switch(on: bool) -> Div {
    let travel = theme::SWITCH_WIDTH - theme::SWITCH_HEIGHT;
    div()
        .relative()
        .flex_none()
        .w(px(theme::SWITCH_WIDTH))
        .h(px(theme::SWITCH_HEIGHT))
        .rounded_full()
        .bg(rgb(if on { theme::SELECTED } else { theme::SWITCH_OFF }))
        .child(
            div()
                .absolute()
                .top(px(theme::SWITCH_KNOB_INSET))
                .left(px(theme::SWITCH_KNOB_INSET + if on { travel } else { 0. }))
                .size(px(theme::SWITCH_HEIGHT - theme::SWITCH_KNOB_INSET * 2.))
                .rounded_full()
                .bg(rgb(theme::BACKGROUND))
                .shadow_sm(),
        )
}

/// 1つだけ選ぶ印。
pub fn radio(selected: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(theme::RADIO_SIZE))
        .rounded_full()
        .bg(rgb(if selected { theme::SELECTED } else { theme::BACKGROUND }))
        .border_1()
        .border_color(rgb(if selected { theme::SELECTED } else { theme::CONTROL_LINE }))
        .when(selected, |mark| mark.child(div().size(px(theme::RADIO_DOT)).rounded_full().bg(rgb(theme::BACKGROUND))))
}

/// スライダーの見た目。`width` は全体の幅、`ratio` は0から1。
pub fn slider_track(id: impl Into<gpui::ElementId>, width: f32, height: f32, ratio: f32) -> Stateful<Div> {
    let knob = theme::SLIDER_KNOB;
    let left = ratio.clamp(0., 1.) * (width - knob);
    let line_top = (height - theme::SLIDER_LINE) / 2.;
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .cursor(gpui::CursorStyle::ResizeLeftRight)
        .child(div().absolute().top(px(line_top)).left_0().right_0().h(px(theme::SLIDER_LINE)).rounded_full().bg(rgba(theme::SLIDER_BACK)))
        .child(div().absolute().top(px(line_top)).left_0().w(px(left + knob / 2.)).h(px(theme::SLIDER_LINE)).rounded_full().bg(rgb(theme::SELECTED)))
        .child(
            div()
                .absolute()
                .top(px((height - knob) / 2.))
                .left(px(left))
                .size(px(knob))
                .rounded_full()
                .bg(rgb(theme::BACKGROUND))
                .border_1()
                .border_color(rgb(theme::CONTROL_LINE))
                .shadow_sm(),
        )
}
