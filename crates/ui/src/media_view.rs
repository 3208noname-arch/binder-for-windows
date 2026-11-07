//! 動画と音声の再生の面。アイコン表示のマウスオーバー、カラムのプレビュー欄、クイックルックで共用する。

use crate::browser::{Browser, window_handle};
use crate::theme;
use filer_core::{
    HOVER_DELAY_MS, HoverGate, PreviewKind, popup_size, SCRUB_RESUME_MS, SCRUB_THROTTLE_MS, SPINNER_DELAY_MS, ViewMode, format_media_time, fraction_at, preview_kind,
};
use filer_shell::{Picture, Player, PlayerOptions, PlayerStatus};
use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::select;
use gpui::{
    Animation, AnimationExt, AnyElement, App, Bounds, Context, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, Pixels, RenderImage, Task, Window, anchored, canvas,
    deferred, div, img, point, prelude::*, px, rgb, rgba,
};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 絵が変わらない時の、進み具合だけの描き直しの間隔。
const PROGRESS: Duration = Duration::from_millis(100);
/// マウスオーバーの面がこの間描かれなければ、見えなくなったとみなして止める。
const UNSEEN: Duration = Duration::from_millis(400);
/// マウスを動かした後に時刻を見せておく長さ。
const TIME_SHOWN: Duration = Duration::from_millis(600);
const CONTROLS_HEIGHT: f32 = 32.;
/// ポップアップとウィンドウの端の間
const POPUP_MARGIN: f32 = 8.;
const GLYPH_FONT: &str = "Segoe MDL2 Assets";
const GLYPH_PLAY: &str = "\u{E768}";
const GLYPH_PAUSE: &str = "\u{E769}";
const GLYPH_SOUND: &str = "\u{E767}";
const GLYPH_MUTED: &str = "\u{E74F}";

/// `Picture` を描ける画像にする。バイトの並びはBGRAのまま渡す。
pub fn picture_image(picture: Picture) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(picture.width, picture.height, picture.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// 測定用の記録。`FILER_MEDIA_LOG` を指定した時だけ書く。
fn media_log(line: String) {
    use std::io::Write;
    if let Some(path) = std::env::var_os("FILER_MEDIA_LOG")
        && let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

type MuteChanged = Rc<dyn Fn(bool, &mut App)>;

/// 1つのファイルの再生。捨てると音が止まる。
pub struct MediaSurface {
    path: PathBuf,
    /// マウスオーバーの見せ方と操作にする
    hover: bool,
    audio: bool,
    muted: bool,
    on_mute: Option<MuteChanged>,
    player: Option<Player>,
    status: PlayerStatus,
    image: Option<Arc<RenderImage>>,
    created: Instant,
    drawn: Instant,
    told: Instant,
    frames: u64,
    area: Rc<Cell<Bounds<Pixels>>>,
    bar: Rc<Cell<Bounds<Pixels>>>,
    last_seek: Option<Instant>,
    resume: Option<Task<()>>,
    time_until: Option<Instant>,
    /// 音声のマウスオーバーの、マウスの位置の割合
    cursor: Option<f64>,
    bar_held: bool,
    /// 前回のマウスの横位置。乗せたままの最初の移動では動かさない
    mouse_x: Option<f32>,
    stopped: bool,
    /// マウスがウィンドウの外にある間は再生しない
    away: bool,
    hwnd: isize,
    pump: Option<Task<()>>,
    wake: mpsc::UnboundedSender<()>,
}

impl MediaSurface {
    pub fn new(path: PathBuf, kind: PreviewKind, hover: bool, muted: bool, on_mute: Option<MuteChanged>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 絵が届いたらすぐ起きる。タイマーの刻みでは60fpsに届かない
        let (wake, mut woken) = mpsc::unbounded::<()>();
        let pump = cx.spawn_in(window, async move |this, cx| {
            loop {
                let timer = cx.background_executor().timer(PROGRESS);
                select(woken.next(), timer).await;
                while woken.try_recv().is_ok() {}
                if this.update_in(cx, |this, window, cx| this.tick(window, cx)).is_err() {
                    break;
                }
            }
        });
        cx.on_release(|this, cx| {
            if let Some(image) = this.image.take() {
                cx.drop_image(image, None);
            }
        })
        .detach();
        let now = Instant::now();
        Self {
            path,
            hover,
            audio: kind == PreviewKind::Audio,
            muted,
            on_mute,
            player: None,
            status: PlayerStatus::default(),
            image: None,
            created: now,
            drawn: now,
            told: now,
            frames: 0,
            area: Rc::default(),
            bar: Rc::default(),
            last_seek: None,
            resume: None,
            time_until: None,
            cursor: None,
            bar_held: false,
            mouse_x: None,
            stopped: false,
            away: false,
            hwnd: window_handle(window),
            pump: Some(pump),
            wake,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 映像の横に対する縦の比。分かるまでは `None`。
    pub fn aspect(&self) -> Option<f32> {
        let (width, height) = self.status.video_size;
        (width > 0 && height > 0).then(|| height as f32 / width as f32)
    }

    /// 動画の絵が出ていて、ポップアップにできる。
    pub fn shows_video(&self) -> bool {
        !self.audio && self.image.is_some()
    }

    /// マウスがウィンドウの外へ出て止まっている。
    fn is_away(&self) -> bool {
        self.away
    }

    pub fn failed(&self) -> bool {
        self.status.failed
    }

    fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.player = None;
        self.stopped = true;
        self.pump = None;
        self.resume = None;
        if let Some(image) = self.image.take() {
            window.drop_image(image).ok();
        }
        cx.notify();
    }

    /// マウスがウィンドウの中にあるか。マウスが外へ出ても一覧には移動が届かないので、自分で調べる。
    fn pointer_inside(&self, window: &Window) -> bool {
        let scale = window.scale_factor();
        let size = window.viewport_size();
        let Some((x, y)) = filer_shell::cursor_position(self.hwnd) else { return false };
        x >= 0 && y >= 0 && (x as f32) < f32::from(size.width) * scale && (y as f32) < f32::from(size.height) * scale
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hover && !self.stopped {
            let away = !self.pointer_inside(window);
            if away != self.away {
                self.away = away;
                self.drawn = Instant::now();
                if away {
                    self.player = None;
                    self.status = PlayerStatus::default();
                    if let Some(image) = self.image.take() {
                        window.drop_image(image).ok();
                    }
                }
                cx.notify();
            }
        }
        let Some(player) = &self.player else { return };
        if self.hover && self.drawn.elapsed() > UNSEEN {
            self.stop(window, cx);
            return;
        }
        let bounds = self.area.get();
        let scale = window.scale_factor();
        player.set_target((f32::from(bounds.size.width) * scale) as u32, (f32::from(bounds.size.height) * scale) as u32);
        let status = player.status();
        if status.ended && !self.status.ended {
            // 終わったら頭に戻して止める。ループする動画はここを通らない
            player.seek(0.);
            player.play(false);
        }
        let mut changed = status.ready != self.status.ready || status.failed != self.status.failed || status.paused != self.status.paused;
        self.status = status;
        if let Some(picture) = player.take_frame()
            && let Some(image) = picture_image(picture)
        {
            if self.frames == 0 {
                media_log(format!("first_frame ms={} {}", self.created.elapsed().as_millis(), self.path.display()));
            }
            self.frames += 1;
            if let Some(old) = self.image.replace(image) {
                window.drop_image(old).ok();
            }
            changed = true;
        }
        if changed || self.told.elapsed() >= PROGRESS {
            self.told = Instant::now();
            cx.notify();
        }
    }

    fn open(&mut self) {
        if self.player.is_some() || self.stopped || self.away {
            return;
        }
        let options = PlayerOptions { looping: self.hover && !self.audio, muted: self.muted && self.hover, autoplay: true };
        let wake = self.wake.clone();
        self.player = Some(Player::open(self.path.clone(), options, move || {
            let _ = wake.unbounded_send(());
        }));
    }

    fn seek_to(&mut self, fraction: f64) {
        if let Some(player) = &self.player {
            player.seek(fraction * self.status.duration);
        }
    }

    /// マウスの横の位置を再生位置にする。動かしている間は止め、止まったら続きを流す。
    fn scrub(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        // 再描画に伴う、位置の変わらない移動は操作ではない
        let x = f32::from(event.position.x);
        if self.mouse_x.replace(x).is_none_or(|last| last == x) {
            return;
        }
        if !self.status.ready || self.status.duration <= 0. {
            return;
        }
        let bounds = self.area.get();
        let fraction = fraction_at(f32::from(event.position.x), f32::from(bounds.origin.x), f32::from(bounds.size.width));
        self.time_until = Some(Instant::now() + TIME_SHOWN);
        if self.audio {
            self.cursor = Some(fraction);
            cx.notify();
            return;
        }
        if self.last_seek.is_some_and(|last| last.elapsed() < Duration::from_millis(SCRUB_THROTTLE_MS)) {
            return;
        }
        self.last_seek = Some(Instant::now());
        self.seek_to(fraction);
        if let Some(player) = &self.player {
            player.play(false);
        }
        self.resume = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(SCRUB_RESUME_MS)).await;
            let _ = this.update(cx, |this, _| {
                if let Some(player) = &this.player {
                    player.play(true);
                }
            });
        }));
        cx.notify();
    }

    /// 音声のマウスオーバーは、押した位置へ飛ぶ。
    fn press_audio(&mut self, event: &MouseDownEvent, _: &mut Window, _: &mut Context<Self>) {
        if self.audio {
            let bounds = self.area.get();
            self.seek_to(fraction_at(f32::from(event.position.x), f32::from(bounds.origin.x), f32::from(bounds.size.width)));
        }
    }

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        if let Some(player) = &self.player {
            player.play(self.status.paused);
        }
        cx.notify();
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.muted = !self.muted;
        if let Some(player) = &self.player {
            player.set_muted(self.muted);
        }
        if let Some(changed) = self.on_mute.clone() {
            changed(self.muted, cx);
        }
        cx.notify();
    }

    fn seek_bar(&mut self, x: f32, cx: &mut Context<Self>) {
        let bar = self.bar.get();
        self.seek_to(fraction_at(x, f32::from(bar.origin.x), f32::from(bar.size.width)));
        cx.notify();
    }

    fn progress(&self) -> f32 {
        if self.status.duration > 0. { (self.status.position / self.status.duration).clamp(0., 1.) as f32 } else { 0. }
    }

    fn time_text(&self) -> String {
        format_media_time(self.status.position, self.status.duration)
    }

    fn waiting(&self) -> bool {
        let shown = if self.audio { self.status.ready } else { self.image.is_some() };
        !shown && !self.status.failed && !self.stopped && self.created.elapsed() >= Duration::from_millis(SPINNER_DELAY_MS)
    }

    fn glyph(glyph: &'static str) -> gpui::Div {
        div().flex().flex_none().items_center().justify_center().size(px(24.)).rounded_sm().font_family(GLYPH_FONT).text_size(px(13.)).child(glyph)
    }

    fn spinner() -> impl IntoElement {
        div().absolute().top_0().bottom_0().left_0().right_0().flex().items_center().justify_center().child(
            div().size(px(12.)).rounded_full().bg(rgb(0xffffff)).with_animation(
                "media-wait",
                Animation::new(Duration::from_millis(900)).repeat(),
                |dot, progress| dot.opacity(0.3 + 0.7 * (1. - (progress * 2. - 1.).abs())),
            ),
        )
    }

    fn area_probe(&self) -> impl IntoElement + use<> {
        let area = self.area.clone();
        canvas(move |bounds, _, _| area.set(bounds), |_, _, _, _| {}).absolute().size_full()
    }

    fn picture(&self) -> Option<impl IntoElement + use<>> {
        self.image.clone().map(|image| img(image).absolute().size_full().object_fit(ObjectFit::Contain))
    }

    fn hover_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let shown = self.image.is_some() || (self.audio && self.status.ready);
        let show_time = self.audio || self.time_until.is_some_and(|until| Instant::now() < until);
        let progress = self.progress();
        let cursor = self.cursor;
        let mute = (!self.audio && shown).then(|| {
            Self::glyph(if self.muted { GLYPH_MUTED } else { GLYPH_SOUND })
                .id("media-mute")
                .absolute()
                .right(px(4.))
                .bottom(px(6.))
                .bg(rgba(0x0000_0099))
                .text_color(rgb(0xffffff))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_mute(cx);
                }))
        });
        div()
            .id("media-hover")
            .relative()
            .size_full()
            .overflow_hidden()
            .rounded_md()
            .when(self.image.is_some(), |surface| surface.bg(rgb(0x000000)))
            .child(self.area_probe())
            .children(self.picture())
            .when(self.waiting(), |surface| surface.child(Self::spinner()))
            .when(shown, |surface| {
                surface
                    .when_some(cursor.filter(|_| self.audio), |surface, cursor| {
                        surface.child(div().absolute().top_0().bottom_0().left(gpui::relative(cursor as f32)).w(px(1.)).bg(rgb(theme::SELECTED)))
                    })
                    .child(div().absolute().left_0().right_0().bottom_0().h(px(3.)).bg(rgba(0xffff_ff55)).child(div().h_full().w(gpui::relative(progress)).bg(rgb(theme::SELECTED))))
                    .when(show_time, |surface| {
                        surface.child(
                            div().absolute().left(px(4.)).bottom(px(6.)).px_1().rounded_sm().bg(rgba(0x0000_0099)).text_color(rgb(0xffffff)).text_size(px(theme::SMALL_TEXT_SIZE)).child(self.time_text()),
                        )
                    })
            })
            .children(mute)
            .on_mouse_move(cx.listener(|this, event, _, cx| this.scrub(event, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::press_audio))
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let bar = self.bar.clone();
        let progress = self.progress();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(CONTROLS_HEIGHT))
            .w_full()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .child(
                Self::glyph(if self.status.paused { GLYPH_PLAY } else { GLYPH_PAUSE })
                    .id("media-play")
                    .hover(|button| button.bg(rgb(theme::HOVER)))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
            )
            .child(
                div()
                    .id("media-bar")
                    .relative()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .h(px(16.))
                    .child(canvas(move |bounds, _, _| bar.set(bounds), |_, _, _, _| {}).absolute().size_full())
                    .child(div().w_full().h(px(4.)).rounded_full().bg(rgb(theme::HOVER)).child(div().h_full().rounded_full().w(gpui::relative(progress)).bg(rgb(theme::SELECTED))))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.bar_held = true;
                            this.seek_bar(f32::from(event.position.x), cx);
                        }),
                    ),
            )
            .child(div().flex_none().text_color(rgb(theme::SUB_TEXT)).child(self.time_text()))
            .child(
                Self::glyph(if self.muted { GLYPH_MUTED } else { GLYPH_SOUND })
                    .id("media-mute")
                    .hover(|button| button.bg(rgb(theme::HOVER)))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
            )
    }

    fn player_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let failed = self.status.failed;
        let stage = (!self.audio && !failed).then(|| {
            div()
                .id("media-stage")
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .bg(rgb(0x000000))
                .child(self.area_probe())
                .children(self.picture())
                .when(self.waiting(), |stage| stage.child(Self::spinner()))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx)))
        });
        // 音声は絵が無くても再生の管理のために枠を覚えておく
        let probe = self.audio.then(|| self.area_probe());
        let body = if failed {
            div().flex().flex_1().items_center().justify_center().text_color(rgb(theme::SUB_TEXT)).child("この形式は再生できません").into_any_element()
        } else {
            self.controls(cx).into_any_element()
        };
        div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .children(probe)
            .children(stage)
            .child(body)
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.bar_held {
                    this.seek_bar(f32::from(event.position.x), cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.bar_held = false))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, _| this.bar_held = false))
    }
}

impl Render for MediaSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drawn = Instant::now();
        self.open();
        if self.hover { self.hover_view(cx).into_any_element() } else { self.player_view(cx).into_any_element() }
    }
}

impl Drop for MediaSurface {
    fn drop(&mut self) {
        media_log(format!("end hover={} frames={} seconds={:.2} fps={:.1}", self.hover, self.frames, self.created.elapsed().as_secs_f32(), self.frames as f32 / self.created.elapsed().as_secs_f32()));
    }
}

/// マウスオーバーの再生の状態。
#[derive(Default)]
pub struct HoverSlot {
    path: Option<PathBuf>,
    surface: Option<Entity<MediaSurface>>,
    timer: Option<Task<()>>,
    /// キーの操作の後は、マウスを動かすまでこの項目を再生しない
    blocked: Option<PathBuf>,
    /// マウスがポップアップの上にある。下のセルからは外れるが、再生は続ける
    over_popup: bool,
}

impl Browser {
    /// 今の状態で再生してよいものだけを残す。どの操作の後でもここを通るので、止め忘れが起きない。
    pub fn sync_media(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let gate = HoverGate {
            enabled: self.settings.hover_play,
            icons: self.tab.view.mode == ViewMode::Icons,
            active: self.window_active,
            busy: self.band.is_some() || self.menu.is_some() || self.tab.renaming.is_some() || cx.has_active_drag(),
            quick_look: self.quick_look.is_some(),
        };
        if self.hover_media.blocked != self.hovered_cell {
            self.hover_media.blocked = None;
        }
        if self.hover_media.surface.as_ref().is_some_and(|surface| surface.read(cx).is_away()) {
            self.hover_media.over_popup = false;
        }
        let hovered = if self.hover_media.over_popup { self.hover_media.path.clone() } else { self.hovered_cell.clone() };
        let wanted = hovered
            .filter(|path| gate.allows() && self.hover_media.blocked.as_ref() != Some(path) && media_kind(path).is_some());
        if wanted != self.hover_media.path {
            self.hover_media.surface = None;
            self.hover_media.timer = None;
            self.hover_media.over_popup = false;
            self.hover_media.path = wanted.clone();
            if let Some(path) = wanted {
                self.hover_media.timer = Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(HOVER_DELAY_MS)).await;
                    let _ = this.update_in(cx, |this, window, cx| this.start_hover(path, window, cx));
                }));
            }
        }
        let open = self.tab.preview.as_ref().filter(|_| self.tab.view.mode == ViewMode::Columns).map(|(folder, entry)| folder.join(entry.os_name()));
        if self.panel_media.as_ref().is_some_and(|surface| Some(surface.read(cx).path()) != open.as_deref()) {
            self.panel_media = None;
        }
        self.sync_quick_look(cx);
    }

    fn start_hover(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.hover_media.path.as_ref() != Some(&path) {
            return;
        }
        let Some(kind) = media_kind(&path) else { return };
        let muted = self.settings.hover_silent;
        let browser = cx.entity().downgrade();
        let on_mute: MuteChanged = Rc::new(move |muted, cx| {
            let _ = browser.update(cx, |browser, cx| browser.set_hover_silent(muted, cx));
        });
        self.hover_media.surface = Some(cx.new(|cx| MediaSurface::new(path, kind, true, muted, Some(on_mute), window, cx)));
        cx.notify();
    }

    /// この項目のマウスオーバーの面。
    pub fn hover_surface(&self, path: &Path) -> Option<Entity<MediaSurface>> {
        self.hover_media.surface.clone().filter(|_| self.hover_media.path.as_deref() == Some(path))
    }

    /// キーの操作の後は、マウスが動くまで今の項目を再生しない。
    pub fn block_hover(&mut self) {
        self.hover_media.blocked = self.hovered_cell.clone();
        self.hover_media.over_popup = false;
    }

    /// 動画を大きめに見せるポップアップ。アイコンの中央に重ね、窓の外へは出さない。
    /// セルと同じ操作を受けるので、押す、開く、掴む、右クリックはそのまま効く。
    pub fn hover_popup(&self, surface: Entity<MediaSurface>, path: PathBuf, is_dir: bool, icon: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let aspect = surface.read(cx).aspect().filter(|_| surface.read(cx).shows_video())?;
        let (width, height) = popup_size(aspect, icon, f32::from(self.viewport.width), f32::from(self.viewport.height));
        let popup = div()
            .id("hover-popup")
            .w(px(width))
            .h(px(height))
            .rounded_lg()
            .overflow_hidden()
            .bg(rgb(0x000000))
            .border_1()
            .border_color(rgb(theme::LINE))
            .shadow_lg()
            .occlude()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hover_media.over_popup = *hovered;
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                // 一覧を流す操作なので、再生をやめて下の一覧へ通す
                this.hover_media.over_popup = false;
                this.hovered_cell = None;
                cx.notify();
            }))
            .child(surface);
        let click_path = path.clone();
        let popup = self
            .item_mouse(popup, path, is_dir, None, cx)
            .on_click(cx.listener(move |this, event, window, cx| this.click_item(click_path.clone(), is_dir, event, window, cx)));
        let offset = point(px((icon - width) / 2.), px((icon - height) / 2.));
        Some(deferred(anchored().offset(offset).snap_to_window_with_margin(px(POPUP_MARGIN)).child(popup)).with_priority(1).into_any_element())
    }

    /// カラムのプレビュー欄で再生を始める。
    pub fn play_in_panel(&mut self, path: PathBuf, kind: PreviewKind, window: &mut Window, cx: &mut Context<Self>) {
        self.panel_media = Some(cx.new(|cx| MediaSurface::new(path, kind, false, false, None, window, cx)));
        cx.notify();
    }

    /// タブを離れる時に、そのタブの再生をすべて止める。
    pub fn stop_media(&mut self) {
        self.hover_media.surface = None;
        self.hover_media.timer = None;
        self.hover_media.path = None;
        self.panel_media = None;
    }
}

/// パスの名前から、再生できる種類なら返す。
fn media_kind(path: &Path) -> Option<PreviewKind> {
    let name = path.file_name()?.to_string_lossy();
    Some(preview_kind(&name, false)).filter(|kind| kind.is_media())
}
