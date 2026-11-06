//! Finderのように細いスクロールバー。gpuiには標準の部品が無いので、位置だけを読んで描く。

use crate::browser::Browser;
use crate::theme;
use filer_core::{MAX_ICON_SIZE, MIN_ICON_SIZE};
use gpui::{
    AnyElement, Context, CursorStyle, MouseButton, MouseDownEvent, MouseMoveEvent, Point, ScrollHandle, div, point,
    prelude::*, px, rgba,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScrollTarget {
    List,
    ListColumns,
    Grid,
    Column(usize),
    Columns,
    Sidebar,
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
    fn scroll_handle(&self, target: ScrollTarget) -> Option<ScrollHandle> {
        match target {
            ScrollTarget::List => Some(self.list_scroll.0.borrow().base_handle.clone()),
            ScrollTarget::ListColumns => Some(self.list_columns_scroll.clone()),
            ScrollTarget::Column(column) => Some(self.columns.get(column)?.scroll.0.borrow().base_handle.clone()),
            ScrollTarget::Columns => Some(self.columns_scroll.clone()),
            ScrollTarget::Sidebar => Some(self.sidebar_scroll.clone()),
            ScrollTarget::Grid => None,
        }
    }

    fn scroll_metrics(&self, target: ScrollTarget, vertical: bool) -> Option<ScrollMetrics> {
        if target == ScrollTarget::Grid {
            let offset: f32 = self.grid_list.scroll_px_offset_for_scrollbar().y.into();
            let max: f32 = self.grid_list.max_offset_for_scrollbar().height.into();
            let viewport: f32 = self.grid_list.viewport_bounds().size.height.into();
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
    pub fn scrollbar(&self, id: &'static str, target: ScrollTarget, vertical: bool, inset: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.bars_visible() {
            return None;
        }
        let metrics = self.scroll_metrics(target, vertical)?;
        let (length, position) = metrics.thumb()?;
        let scale = metrics.scale();
        let thumb = div()
            .id(id)
            .absolute()
            .rounded_full()
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
        let offset = -offset.max(0.0);
        if drag.target == ScrollTarget::Grid {
            self.grid_list.set_offset_from_scrollbar(point(px(0.), px(offset)));
        } else if let Some(handle) = self.scroll_handle(drag.target) {
            let current = handle.offset();
            handle.set_offset(if drag.vertical { point(current.x, px(offset)) } else { point(px(offset), current.y) });
        }
        cx.notify();
    }

    /// 下の帯のアイコンの大きさの部品。
    pub fn icon_size_control(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let size = self.view.icon_size;
        let ratio = (size - MIN_ICON_SIZE) / (MAX_ICON_SIZE - MIN_ICON_SIZE);
        let step = |id: &'static str, label: &'static str, delta: f32, cx: &mut Context<Self>| {
            div()
                .id(id)
                .px_1()
                .cursor(CursorStyle::PointingHand)
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let size = (this.view.icon_size + delta).clamp(MIN_ICON_SIZE, MAX_ICON_SIZE);
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
        self.view.icon_size = (drag.start_size + moved * per_pixel).clamp(MIN_ICON_SIZE, MAX_ICON_SIZE);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::ScrollMetrics;

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
