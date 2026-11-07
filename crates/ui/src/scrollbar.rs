//! Finderのように細いスクロールバー。gpuiには標準の部品が無いので、位置だけを読んで描く。

use crate::browser::Browser;
use crate::theme;
use filer_core::{MAX_ICON_SIZE, MIN_ICON_SIZE};
use filer_core::{ScrollbarMode, ViewMode};
use gpui::{
    AnyElement, Bounds, Context, CursorStyle, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, ScrollHandle, div, point,
    prelude::*, px, rgba,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollTarget {
    List,
    ListColumns,
    Grid,
    Column(usize),
    Columns,
    Sidebar,
}

impl ScrollTarget {
    /// 列の数によらず、1つずつある対象。
    pub const FIXED: [ScrollTarget; 5] =
        [ScrollTarget::List, ScrollTarget::ListColumns, ScrollTarget::Grid, ScrollTarget::Columns, ScrollTarget::Sidebar];

    fn vertical(self) -> bool {
        !matches!(self, ScrollTarget::ListColumns | ScrollTarget::Columns)
    }
}

/// 1本のスクロールバーの出入り。スクロールされた時に現れ、止まって少しすると消える。
#[derive(Clone, Copy, Default)]
pub struct BarFade {
    offset: Option<f32>,
    appeared: Option<Instant>,
    moved: Option<Instant>,
}

impl BarFade {
    const FADE_IN: Duration = Duration::from_millis(theme::SCROLLBAR_FADE_IN_MS);
    const FADE_OUT: Duration = Duration::from_millis(theme::SCROLLBAR_FADE_OUT_MS);
    const LINGER: Duration = Duration::from_millis(theme::SCROLLBAR_LINGER_MS);

    /// 今の位置を伝える。最初の1回は基準にするだけで、バーは出さない。
    pub fn observe(&mut self, offset: f32, now: Instant) {
        if self.offset.is_some_and(|last| last != offset) {
            if self.opacity(now) <= 0. {
                self.appeared = Some(now);
            }
            self.moved = Some(now);
        }
        self.offset = Some(offset);
    }

    pub fn opacity(&self, now: Instant) -> f32 {
        let (Some(appeared), Some(moved)) = (self.appeared, self.moved) else { return 0. };
        let fade_in = now.saturating_duration_since(appeared).as_secs_f32() / Self::FADE_IN.as_secs_f32();
        let idle = now.saturating_duration_since(moved).saturating_sub(Self::LINGER);
        let fade_out = 1. - idle.as_secs_f32() / Self::FADE_OUT.as_secs_f32();
        fade_in.min(fade_out).clamp(0., 1.)
    }

    /// 濃さが変わっている最中か。
    pub fn fading(&self, now: Instant) -> bool {
        let (Some(appeared), Some(moved)) = (self.appeared, self.moved) else { return false };
        let fade_out = moved + Self::LINGER;
        now < appeared + Self::FADE_IN || (fade_out <= now && now < fade_out + Self::FADE_OUT)
    }

    /// 出ているバーが薄くなり始める時刻。
    pub fn fade_start(&self, now: Instant) -> Option<Instant> {
        self.moved.map(|moved| moved + Self::LINGER).filter(|start| *start > now)
    }
}

/// スクロールの位置。`offset` は上端からの距離で、0以上。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMetrics {
    pub offset: f32,
    pub max: f32,
    pub viewport: f32,
}

impl ScrollMetrics {
    /// つまみの長さと位置。スクロールできない時は `None`。
    pub fn thumb(&self) -> Option<(f32, f32)> {
        if self.max < 1.0 || self.viewport < 1.0 {
            return None;
        }
        let length = (self.viewport * self.viewport / (self.viewport + self.max)).max(theme::SCROLL_MIN_THUMB);
        let travel = self.viewport - length;
        Some((length, travel * (self.offset / self.max).clamp(0.0, 1.0)))
    }

    /// つまみを1ピクセル動かした時のスクロール量。
    fn scale(&self) -> f32 {
        let (length, _) = self.thumb().unwrap_or((self.viewport, 0.0));
        self.max / (self.viewport - length).max(1.0)
    }
}

/// つまみをドラッグしている最中の状態。
#[derive(Clone, Copy)]
pub struct ScrollDrag {
    target: ScrollTarget,
    vertical: bool,
    start: f32,
    start_offset: f32,
    scale: f32,
}

/// アイコンの大きさのつまみをドラッグしている最中の状態。
#[derive(Clone, Copy)]
pub struct IconSizeDrag {
    start_x: f32,
    start_size: f32,
}

fn along(point: Point<gpui::Pixels>, vertical: bool) -> f32 {
    (if vertical { point.y } else { point.x }).into()
}

impl Browser {
    /// スクロールの位置。スクロールされたかの判断に使う。
    pub fn bar_offset(&self, target: ScrollTarget) -> f32 {
        self.scroll_metrics(target, target.vertical()).map_or(0., |metrics| metrics.offset)
    }

    fn bar_opacity(&self, target: ScrollTarget) -> f32 {
        if self.settings.scrollbar == ScrollbarMode::Always || self.scroll_drag.is_some_and(|drag| drag.target == target) {
            return 1.;
        }
        let bar = match target {
            ScrollTarget::Column(index) => self.tab.columns.get(index).map(|column| &column.bar),
            _ => self.tab.bars.get(&target),
        };
        bar.map_or(0., |bar| bar.opacity(Instant::now()))
    }

    fn scroll_handle(&self, target: ScrollTarget) -> Option<ScrollHandle> {
        match target {
            ScrollTarget::List => Some(self.tab.list_scroll.0.borrow().base_handle.clone()),
            ScrollTarget::ListColumns => Some(self.tab.list_columns_scroll.clone()),
            ScrollTarget::Column(column) => Some(self.tab.columns.get(column)?.scroll.0.borrow().base_handle.clone()),
            ScrollTarget::Columns => Some(self.tab.columns_scroll.clone()),
            ScrollTarget::Sidebar => Some(self.sidebar_scroll.clone()),
            ScrollTarget::Grid => None,
        }
    }

    fn scroll_metrics(&self, target: ScrollTarget, vertical: bool) -> Option<ScrollMetrics> {
        if target == ScrollTarget::Grid {
            let offset: f32 = self.tab.grid_list.scroll_px_offset_for_scrollbar().y.into();
            let max: f32 = self.tab.grid_list.max_offset_for_scrollbar().height.into();
            let viewport: f32 = self.tab.grid_list.viewport_bounds().size.height.into();
            return Some(ScrollMetrics { offset: -offset, max, viewport });
        }
        let handle = self.scroll_handle(target)?;
        let bounds = handle.bounds().size;
        let (offset, max, viewport) = if vertical {
            (handle.offset().y, handle.max_offset().height, bounds.height)
        } else {
            (handle.offset().x, handle.max_offset().width, bounds.width)
        };
        let offset: f32 = offset.into();
        Some(ScrollMetrics { offset: -offset, max: max.into(), viewport: viewport.into() })
    }

    /// つまみを描く。親の要素は `relative` で、`inset` はつまみの動く範囲の始まり。
    pub fn scrollbar(&self, id: impl Into<gpui::ElementId>, target: ScrollTarget, vertical: bool, inset: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let opacity = self.bar_opacity(target);
        if opacity <= 0. {
            return None;
        }
        let metrics = self.scroll_metrics(target, vertical)?;
        let (length, position) = metrics.thumb()?;
        let scale = metrics.scale();
        let thumb = div()
            .id(id)
            .absolute()
            .rounded_full()
            .opacity(opacity)
            .bg(rgba(theme::SCROLL_THUMB))
            .hover(|s| s.bg(rgba(theme::SCROLL_THUMB_HOVER)))
            .cursor(CursorStyle::Arrow)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.menu = None;
                    let start = along(event.position, vertical);
                    this.scroll_drag = Some(ScrollDrag { target, vertical, start, start_offset: metrics.offset, scale });
                }),
            );
        let thumb = if vertical {
            thumb.right(px(theme::SCROLL_MARGIN)).top(px(inset + position)).w(px(theme::SCROLL_WIDTH)).h(px(length))
        } else {
            thumb.bottom(px(theme::SCROLL_MARGIN)).left(px(inset + position)).h(px(theme::SCROLL_WIDTH)).w(px(length))
        };
        Some(thumb.into_any_element())
    }

    pub fn drag_scroll(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.scroll_drag else { return };
        let offset = drag.start_offset + (along(event.position, drag.vertical) - drag.start) * drag.scale;
        self.set_scroll(drag.target, drag.vertical, offset);
        cx.notify();
    }

    fn set_scroll(&mut self, target: ScrollTarget, vertical: bool, offset: f32) {
        let offset = -offset.max(0.0);
        if target == ScrollTarget::Grid {
            self.tab.grid_list.set_offset_from_scrollbar(point(px(0.), px(offset)));
        } else if let Some(handle) = self.scroll_handle(target) {
            let current = handle.offset();
            handle.set_offset(if vertical { point(current.x, px(offset)) } else { point(px(offset), current.y) });
        }
    }

    /// カーソルの下の、縦に動く一覧。
    fn scroll_target_at(&self, position: Point<Pixels>) -> Option<(ScrollTarget, Bounds<Pixels>)> {
        let fixed = |target| self.scroll_handle(target).map(|handle| (target, handle.bounds()));
        if f32::from(position.x) < theme::SIDEBAR_WIDTH {
            return fixed(ScrollTarget::Sidebar);
        }
        match self.tab.view.mode {
            ViewMode::List => fixed(ScrollTarget::List),
            ViewMode::Icons => Some((ScrollTarget::Grid, self.tab.grid_list.viewport_bounds())),
            ViewMode::Columns => (0..self.tab.columns.len())
                .filter_map(|index| fixed(ScrollTarget::Column(index)))
                .find(|(_, bounds)| bounds.left() <= position.x && position.x < bounds.right()),
        }
    }

    /// ドラッグ中のスクロール。ホイールの回転と、一覧の上下の端で止まった時に送る。
    pub fn scroll_while_dragging(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let now = Instant::now();
        let elapsed = self.drag_tick.replace(now).map_or(0., |last| now.duration_since(last).as_secs_f32().min(0.1));
        let wheel = filer_shell::take_wheel();
        let Some((target, bounds)) = self.scroll_target_at(position) else { return };
        // 下の端は、ドラッグ中の帯の上までにする
        let floor = f32::from(self.viewport.height) - theme::STATUS_HEIGHT - theme::DRAG_BAR_HEIGHT;
        let (top, bottom) = (f32::from(bounds.top()), f32::from(bounds.bottom()).min(floor));
        let y = f32::from(position.y);
        let mut delta = -(wheel as f32) / 120. * theme::DRAG_WHEEL_STEP;
        if y >= top - theme::ROW_HEIGHT && y < top + theme::DRAG_EDGE_WIDTH {
            delta -= theme::DRAG_EDGE_SPEED * elapsed;
        } else if y > bottom - theme::DRAG_EDGE_WIDTH && y <= bottom {
            delta += theme::DRAG_EDGE_SPEED * elapsed;
        }
        if delta == 0. {
            return;
        }
        let Some(metrics) = self.scroll_metrics(target, true) else { return };
        let offset = (metrics.offset + delta).clamp(0., metrics.max);
        if offset != metrics.offset {
            self.set_scroll(target, true, offset);
            cx.notify();
        }
    }

    /// 下の帯のアイコンの大きさの部品。
    pub fn icon_size_control(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let size = self.tab.view.icon_size;
        let ratio = (size - MIN_ICON_SIZE) / (MAX_ICON_SIZE - MIN_ICON_SIZE);
        let step = |id: &'static str, label: &'static str, delta: f32, cx: &mut Context<Self>| {
            div()
                .id(id)
                .px_1()
                .cursor(CursorStyle::PointingHand)
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let size = (this.tab.view.icon_size + delta).clamp(MIN_ICON_SIZE, MAX_ICON_SIZE);
                    this.update_view(cx, |view| view.icon_size = size);
                }))
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(step("icon-smaller", "−", -theme::ICON_SIZE_STEP, cx))
            .child(
                div()
                    .id("icon-size")
                    .relative()
                    .w(px(theme::ICON_SLIDER_WIDTH))
                    .h(px(theme::STATUS_HEIGHT))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .child(div().absolute().top(px(11.)).left_0().right_0().h(px(3.)).rounded_full().bg(rgba(theme::SCROLL_THUMB)))
                    .child(
                        div()
                            .absolute()
                            .top(px(7.))
                            .left(px(ratio * (theme::ICON_SLIDER_WIDTH - 12.)))
                            .size(px(12.))
                            .rounded_full()
                            .bg(rgba(theme::ICON_SLIDER_KNOB)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.icon_size_drag = Some(IconSizeDrag { start_x: event.position.x.into(), start_size: size });
                        }),
                    ),
            )
            .child(step("icon-larger", "＋", theme::ICON_SIZE_STEP, cx))
    }

    pub fn drag_icon_size(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.icon_size_drag else { return };
        let moved = f32::from(event.position.x) - drag.start_x;
        let per_pixel = (MAX_ICON_SIZE - MIN_ICON_SIZE) / (theme::ICON_SLIDER_WIDTH - 12.);
        self.tab.view.icon_size = (drag.start_size + moved * per_pixel).clamp(MIN_ICON_SIZE, MAX_ICON_SIZE);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{BarFade, ScrollMetrics};
    use std::time::{Duration, Instant};

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn first_position_does_not_show_the_bar() {
        let (mut bar, now) = (BarFade::default(), Instant::now());
        bar.observe(120., now);
        bar.observe(120., now + ms(50));
        assert_eq!(bar.opacity(now + ms(50)), 0.);
    }

    #[test]
    fn bar_fades_in_stays_and_fades_out() {
        let (mut bar, now) = (BarFade::default(), Instant::now());
        bar.observe(0., now);
        bar.observe(10., now);
        assert_eq!(bar.opacity(now), 0.);
        assert!(bar.fading(now), "濃さが0の瞬間も、描き続けないと現れない");
        assert!(bar.fading(now + ms(BarFade::FADE_IN.as_millis() as u64 / 2)));
        assert!(!bar.fading(now + BarFade::FADE_IN));
        assert_eq!(bar.opacity(now + BarFade::FADE_IN), 1.);
        assert_eq!(bar.opacity(now + BarFade::LINGER), 1.);
        assert!(bar.fading(now + BarFade::LINGER + BarFade::FADE_OUT / 2));
        assert_eq!(bar.opacity(now + BarFade::LINGER + BarFade::FADE_OUT), 0.);
        assert!(!bar.fading(now + BarFade::LINGER + BarFade::FADE_OUT));
    }

    #[test]
    fn scrolling_again_keeps_the_bar() {
        let (mut bar, now) = (BarFade::default(), Instant::now());
        bar.observe(0., now);
        bar.observe(10., now);
        let later = now + BarFade::LINGER;
        bar.observe(20., later);
        assert_eq!(bar.opacity(later + BarFade::LINGER), 1.);
    }

    #[test]
    fn no_thumb_when_everything_fits() {
        assert_eq!(ScrollMetrics { offset: 0.0, max: 0.0, viewport: 100.0 }.thumb(), None);
    }

    #[test]
    fn thumb_length_follows_visible_share() {
        let (length, position) = ScrollMetrics { offset: 0.0, max: 1200.0, viewport: 400.0 }.thumb().unwrap();
        assert_eq!(length, 100.0);
        assert_eq!(position, 0.0);
    }

    #[test]
    fn thumb_reaches_the_end_at_max_offset() {
        let (length, position) = ScrollMetrics { offset: 300.0, max: 300.0, viewport: 100.0 }.thumb().unwrap();
        assert_eq!(length + position, 100.0);
    }

    #[test]
    fn huge_lists_keep_a_grabbable_thumb() {
        let (length, _) = ScrollMetrics { offset: 0.0, max: 2_000_000.0, viewport: 500.0 }.thumb().unwrap();
        assert_eq!(length, crate::theme::SCROLL_MIN_THUMB);
    }
}
