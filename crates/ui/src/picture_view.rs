//! クイックルックの画像。枠に合わせて見せ、拡大した時はドラッグとホイールで見る場所を動かす。
//! 倍率は「元の1画素が画面の何画素になるか」で表し、Windowsの拡大率は掛けない。

use crate::media_view::picture_image;
use crate::theme;
use filer_core::{clamp_offset, fit_scale, zoom_label, zoom_offset, zoom_step};
use filer_shell::decode_image_sized;
use gpui::{
    Bounds, Context, CursorStyle, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, Pixels, Point, RenderImage, ScrollDelta, ScrollWheelEvent, Task, Window, canvas, div, img,
    point, prelude::*, px, rgb, rgba,
};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// 元の大きさで読む時の長辺の上限。gpuiが描ける絵の上限に合わせる。
const FULL_LIMIT: u32 = 16384;
/// 縮小図の長辺
const MAP_SIZE: f32 = 120.;
const MAP_MARGIN: f32 = 12.;
/// ホイール1行で動かす量（論理ピクセル）
const WHEEL_LINE: f32 = 40.;

pub struct PictureView {
    path: PathBuf,
    /// 長辺を縮めて読んだ絵。枠に合わせる時と縮小図に使う
    base: Arc<RenderImage>,
    /// 元の大きさの絵。細かい倍率になった時に読む
    full: Option<Arc<RenderImage>>,
    full_load: Option<Task<()>>,
    /// 向きを直した後の元の縦横（画素）
    source: (f32, f32),
    /// 倍率。`None` は枠に合わせる
    scale: Option<f32>,
    /// 画像の左上の、枠の左上からの位置（論理ピクセル）
    offset: Point<f32>,
    view: Rc<Cell<Bounds<Pixels>>>,
    map: Rc<Cell<Bounds<Pixels>>>,
    /// ドラッグで動かしている間の、前回のマウスの位置
    drag: Option<Point<Pixels>>,
    map_drag: bool,
    /// Ctrl+ホイールの、まだ倍率に使っていない回転量
    wheel: f32,
}

impl PictureView {
    pub fn new(path: PathBuf, base: Arc<RenderImage>, source: (u32, u32), cx: &mut Context<Self>) -> Self {
        cx.on_release(|this, cx| {
            cx.drop_image(this.base.clone(), None);
            if let Some(full) = this.full.take() {
                cx.drop_image(full, None);
            }
        })
        .detach();
        Self {
            path,
            base,
            full: None,
            full_load: None,
            source: (source.0 as f32, source.1 as f32),
            scale: None,
            offset: point(0., 0.),
            view: Rc::default(),
            map: Rc::default(),
            drag: None,
            map_drag: false,
            wheel: 0.,
        }
    }

    /// 縮めて読んだ絵の縦横。ウィンドウの形を決めるのに使う。
    pub fn base_image(&self) -> &Arc<RenderImage> {
        &self.base
    }

    fn view_size(&self) -> (f32, f32) {
        let size = self.view.get().size;
        (f32::from(size.width), f32::from(size.height))
    }

    fn current_scale(&self, window: &Window) -> f32 {
        let sf = window.scale_factor();
        let (width, height) = self.view_size();
        self.scale.unwrap_or_else(|| fit_scale(self.source, (width * sf, height * sf)))
    }

    /// 今の倍率での画像の大きさ（論理ピクセル）と、枠に収めた左上の位置。
    fn layout(&self, window: &Window) -> ((f32, f32), Point<f32>) {
        let scale = self.current_scale(window) / window.scale_factor();
        let size = (self.source.0 * scale, self.source.1 * scale);
        let (width, height) = self.view_size();
        (size, point(clamp_offset(self.offset.x, size.0, width), clamp_offset(self.offset.y, size.1, height)))
    }

    /// 画像が枠からはみ出していて、動かして見る状態か。
    fn pannable(&self, window: &Window) -> bool {
        let ((width, height), _) = self.layout(window);
        let (view_width, view_height) = self.view_size();
        self.scale.is_some() && (width > view_width + 0.5 || height > view_height + 0.5)
    }

    /// 倍率を変える。`anchor` は動かさない点（ウィンドウの座標）で、無ければ枠の中央にする。
    fn set_scale(&mut self, scale: Option<f32>, anchor: Option<Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.view.get();
        if view.size.width <= px(0.) {
            return;
        }
        let old = self.current_scale(window);
        let (_, offset) = self.layout(window);
        self.scale = scale;
        let new = self.current_scale(window);
        let anchor = anchor.unwrap_or(view.center()) - view.origin;
        self.offset = point(zoom_offset(offset.x, f32::from(anchor.x), old, new), zoom_offset(offset.y, f32::from(anchor.y), old, new));
        self.load_full(new, window, cx);
        cx.notify();
    }

    pub fn zoom(&mut self, zoom_in: bool, window: &mut Window, cx: &mut Context<Self>) {
        let next = zoom_step(self.current_scale(window), zoom_in);
        self.set_scale(Some(next), None, window, cx);
    }

    pub fn actual_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_scale(Some(1.), None, window, cx);
    }

    pub fn fit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_scale(None, None, window, cx);
    }

    /// 枠に合わせると等倍を往復する。
    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scale.is_none() { self.actual_size(window, cx) } else { self.fit(window, cx) }
    }

    /// 縮めて読んだ絵では粗くなる倍率なら、元の大きさで読み直す。
    fn load_full(&mut self, scale: f32, window: &mut Window, cx: &mut Context<Self>) {
        let base_width = self.base.size(0).width.0 as f32;
        if self.full.is_some() || self.full_load.is_some() || self.source.0 * scale <= base_width + 0.5 || self.source.0 <= base_width {
            return;
        }
        let path = self.path.clone();
        self.full_load = Some(cx.spawn_in(window, async move |this, cx| {
            let picture = cx.background_executor().spawn(async move { decode_image_sized(&path, FULL_LIMIT, FULL_LIMIT) }).await;
            let _ = this.update(cx, |this, cx| {
                this.full = picture.ok().and_then(|(picture, _)| picture_image(picture));
                cx.notify();
            });
        }));
    }

    fn move_by(&mut self, dx: f32, dy: f32, window: &Window, cx: &mut Context<Self>) {
        let (_, offset) = self.layout(window);
        self.offset = point(offset.x + dx, offset.y + dy);
        cx.notify();
    }

    /// 縮小図の上の位置が、枠の中央に来るように動かす。
    fn center_on_map(&mut self, position: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let map = self.map.get();
        if map.size.width <= px(0.) {
            return;
        }
        let fx = (f32::from(position.x - map.origin.x) / f32::from(map.size.width)).clamp(0., 1.);
        let fy = (f32::from(position.y - map.origin.y) / f32::from(map.size.height)).clamp(0., 1.);
        let ((width, height), _) = self.layout(window);
        let (view_width, view_height) = self.view_size();
        self.offset = point(view_width / 2. - fx * width, view_height / 2. - fy * height);
        cx.notify();
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.map.get().contains(&event.position) && self.pannable(window) {
            self.map_drag = true;
            self.center_on_map(event.position, window, cx);
        } else if self.pannable(window) {
            self.drag = Some(event.position);
            cx.notify();
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag = None;
            self.map_drag = false;
            return;
        }
        if self.map_drag {
            self.center_on_map(event.position, window, cx);
        } else if let Some(last) = self.drag.replace(event.position) {
            let delta = event.position - last;
            self.move_by(f32::from(delta.x), f32::from(delta.y), window, cx);
        }
    }

    fn mouse_up(&mut self, cx: &mut Context<Self>) {
        self.drag = None;
        self.map_drag = false;
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (dx, dy) = match event.delta {
            ScrollDelta::Lines(lines) => (lines.x * WHEEL_LINE, lines.y * WHEEL_LINE),
            ScrollDelta::Pixels(pixels) => (f32::from(pixels.x), f32::from(pixels.y)),
        };
        if event.modifiers.control || event.modifiers.alt {
            // 1ノッチで1段。トラックパッドの細かい回転は溜めてから動かす
            self.wheel += dy;
            if self.wheel.abs() < WHEEL_LINE {
                return;
            }
            let next = zoom_step(self.current_scale(window), self.wheel > 0.);
            self.wheel = 0.;
            self.set_scale(Some(next), Some(event.position), window, cx);
        } else if self.pannable(window) {
            self.move_by(dx, dy, window, cx);
        }
    }

    fn navigator(&self, size: (f32, f32), offset: Point<f32>) -> impl IntoElement + use<> {
        let long = self.source.0.max(self.source.1);
        let (map_width, map_height) = (MAP_SIZE * self.source.0 / long, MAP_SIZE * self.source.1 / long);
        let (view_width, view_height) = self.view_size();
        let left = (-offset.x / size.0).clamp(0., 1.);
        let top = (-offset.y / size.1).clamp(0., 1.);
        let width = (view_width / size.0).min(1. - left);
        let height = (view_height / size.1).min(1. - top);
        let map = self.map.clone();
        div()
            .absolute()
            .right(px(MAP_MARGIN))
            .bottom(px(MAP_MARGIN))
            .w(px(map_width))
            .h(px(map_height))
            .bg(rgba(0x0000_0099))
            .border_1()
            .border_color(rgb(theme::LINE))
            .shadow_md()
            .child(canvas(move |bounds, _, _| map.set(bounds), |_, _, _, _| {}).absolute().size_full())
            .child(img(self.base.clone()).size_full())
            .child(
                div()
                    .absolute()
                    .left(px(left * map_width))
                    .top(px(top * map_height))
                    .w(px(width * map_width))
                    .h(px(height * map_height))
                    .border_1()
                    .border_color(rgb(theme::SELECTED))
                    .bg(rgba(0xffff_ff22)),
            )
    }
}

impl Render for PictureView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.view.clone();
        // 枠の大きさが変わったら描き直す。描いている途中は描き直しを頼めないので、次のコマで頼む
        let probe = canvas(
            move |bounds, window, _| {
                if view.replace(bounds) != bounds {
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let measured = self.view.get().size.width > px(0.);
        let (size, offset) = self.layout(window);
        self.offset = offset;
        let pannable = measured && self.pannable(window);
        let drawn = self.full.clone().unwrap_or_else(|| self.base.clone());
        let picture = if measured {
            img(drawn).absolute().left(px(offset.x)).top(px(offset.y)).w(px(size.0)).h(px(size.1)).into_any_element()
        } else {
            img(drawn).size_full().object_fit(ObjectFit::Contain).into_any_element()
        };
        let label = self.scale.map(|_| {
            div()
                .absolute()
                .left(px(MAP_MARGIN))
                .bottom(px(MAP_MARGIN))
                .px_1()
                .rounded_sm()
                .bg(rgba(0x0000_0099))
                .text_color(rgb(0xffffff))
                .text_size(px(theme::SMALL_TEXT_SIZE))
                .child(zoom_label(self.current_scale(window)))
        });
        let cursor = if self.drag.is_some() { CursorStyle::ClosedHand } else { CursorStyle::OpenHand };
        div()
            .id("quick-look-picture")
            .relative()
            .size_full()
            .overflow_hidden()
            .child(probe)
            .child(picture)
            .when(pannable, |view| view.cursor(cursor).child(self.navigator(size, offset)))
            .children(label)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| this.mouse_up(cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| this.mouse_up(cx)))
            .on_scroll_wheel(cx.listener(Self::scroll))
    }
}
