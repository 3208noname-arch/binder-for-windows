//! サイドバーの「よく使う項目」。フォルダのドラッグでの登録、行のドラッグでの並べ替え、右クリックの操作。

use crate::actions::SpringTarget;
use crate::browser::{Browser, Menu};
use crate::text_input::{InputEvent, TextInput};
use crate::theme;
use filer_core::{DropZone, Favorite, Gesture, add_favorites, drop_zone, place_favorites, remove_favorite, rename_favorite};
use gpui::{
    AnyElement, Bounds, Context, CursorStyle, DragMoveEvent, Entity, ExternalPaths, Focusable, MouseButton, MouseDownEvent, Pixels, Point,
    Render, Stateful, Subscription, Window, div, prelude::*, px, rgb,
};
use std::path::{Path, PathBuf};

/// ドラッグが指している行の間。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapOwner {
    Row(usize),
    Heading(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FavoriteGap {
    owner: GapOwner,
    gap: usize,
}

/// 並べ替えで運んでいる行。
struct FavoriteDrag {
    path: PathBuf,
    name: String,
}

/// カーソルに付ける札。
struct DragLabel(String);

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_0p5()
            .rounded_md()
            .bg(rgb(theme::SELECTED))
            .text_color(rgb(theme::SELECTED_TEXT))
            .text_size(px(theme::TEXT_SIZE))
            .font_family(theme::FONT)
            .child(self.0.clone())
    }
}

pub struct FavoriteRename {
    path: PathBuf,
    pub input: Entity<TextInput>,
    _subscriptions: [Subscription; 2],
}

impl Browser {
    /// 保存された並びへ変更を加えて書き戻す。別のウィンドウでの変更を消さないよう、毎回読み直す。
    pub fn edit_favorites(&mut self, edit: impl FnOnce(&mut Vec<Favorite>), cx: &mut Context<Self>) {
        if let Some(saved) = self.store.as_ref().and_then(|store| store.favorites()) {
            self.favorites = saved;
        }
        edit(&mut self.favorites);
        let saved = self.store.as_ref().map_or(Ok(()), |store| store.save_favorites(&self.favorites));
        if let Err(error) = saved {
            self.show_message(format!("よく使う項目を保存できませんでした: {error}"), cx);
        }
        cx.notify();
    }

    pub fn add_favorites(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.edit_favorites(|list| add_favorites(list, &paths), cx);
    }

    /// 運ばれてきた項目のうち、フォルダだけを登録する。切断中のNASでも画面を待たせない。
    fn register_dropped(&mut self, paths: Vec<PathBuf>, gap: usize, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let folders: Vec<PathBuf> = cx.background_spawn(async move { paths.into_iter().filter(|path| path.is_dir()).collect() }).await;
            let _ = this.update(cx, |browser, cx| {
                if folders.is_empty() {
                    browser.show_message("よく使う項目に登録できるのはフォルダだけです".into(), cx);
                } else {
                    browser.edit_favorites(|list| place_favorites(list, &folders, gap), cx);
                }
            });
        })
        .detach();
    }

    /// ドラッグの位置から行の間を求める。行の間を指していれば真。
    fn track_gap(&mut self, owner: GapOwner, bounds: Bounds<Pixels>, position: Point<Pixels>, into: bool, cx: &mut Context<Self>) -> bool {
        let gap = bounds.contains(&position).then(|| match owner {
            GapOwner::Heading(gap) => Some(gap),
            GapOwner::Row(row) => {
                let (offset, height) = ((position.y - bounds.top()) / px(1.), bounds.size.height / px(1.));
                match drop_zone(offset, height) {
                    DropZone::Before => Some(row),
                    DropZone::After => Some(row + 1),
                    DropZone::Into if into => None,
                    DropZone::Into => Some(if offset < height / 2. { row } else { row + 1 }),
                }
            }
        });
        match gap.flatten() {
            Some(gap) => {
                let next = Some(FavoriteGap { owner, gap });
                if self.favorite_gap != next {
                    self.favorite_gap = next;
                    cx.notify();
                }
                true
            }
            None => {
                if self.favorite_gap.is_some_and(|gap| gap.owner == owner) {
                    self.favorite_gap = None;
                    cx.notify();
                }
                false
            }
        }
    }

    fn take_gap(&mut self) -> Option<usize> {
        self.favorite_gap.take().map(|gap| gap.gap)
    }

    /// ドラッグの札や帯を出すのは項目を運ぶ時だけで、行の並べ替えは含めない。
    pub fn dragging_items(&self, cx: &Context<Self>) -> bool {
        cx.has_active_drag() && !self.reordering_favorite
    }

    /// ドラッグが終わった時の片付け。
    pub fn clear_favorite_drag(&mut self) {
        self.favorite_gap = None;
        self.reordering_favorite = false;
    }

    /// サイドバーの1行。`favorite` はよく使う項目の行の番号。
    pub fn sidebar_item(&self, id: String, label: AnyElement, path: PathBuf, favorite: Option<usize>, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let active = self.tab.history.current() == path;
        let fill = self.drag_fill(&SpringTarget::Go(path.clone()));
        let in_gap = favorite.is_some_and(|row| self.favorite_gap.is_some_and(|gap| gap.owner == GapOwner::Row(row)));
        let (hover_path, drop_path, tab_path, click_path) = (path.clone(), path.clone(), path.clone(), path.clone());
        div()
            .id(gpui::ElementId::Name(id.into()))
            .px_2()
            .py_0p5()
            .rounded_md()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .cursor(CursorStyle::PointingHand)
            .when(active, |b| b.bg(rgb(theme::HOVER)))
            .hover(|s| s.bg(rgb(theme::HOVER)))
            .child(label)
            .drag_over::<ExternalPaths>(move |style, _, _, _| if in_gap { style } else { Self::drag_style(style, fill) })
            .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                let position = event.event.position;
                let gap = favorite.is_some_and(|row| this.track_gap(GapOwner::Row(row), event.bounds, position, true, cx));
                // 行の間を指している間は、フォルダを開く待ちを止める
                this.hover_target(SpringTarget::Go(hover_path.clone()), event.bounds, if gap { Point::default() } else { position }, cx);
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                cx.stop_propagation();
                if let Some(gap) = this.take_gap() {
                    this.end_spring();
                    this.register_dropped(paths.paths().to_vec(), gap, cx);
                    return;
                }
                let copy = filer_shell::control_down();
                this.drop_paths(paths.paths(), drop_path.clone(), copy, cx);
            }))
            .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, _, cx| {
                this.item_gesture(Gesture::MiddleClick, tab_path.clone(), true, cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.renaming_favorite(&click_path) {
                    this.navigate(click_path.clone(), cx);
                }
            }))
    }

    fn renaming_favorite(&self, path: &Path) -> bool {
        self.favorite_rename.as_ref().is_some_and(|rename| rename.path == path)
    }

    fn favorite_row(&self, row: usize, favorite: &Favorite, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let id = format!("fav-{}", favorite.path.display());
        let renaming = self.renaming_favorite(&favorite.path);
        let label = match &self.favorite_rename {
            Some(rename) if renaming => rename.input.clone().into_any_element(),
            _ => favorite.name.clone().into_any_element(),
        };
        let browser = cx.weak_entity();
        let drag = FavoriteDrag { path: favorite.path.clone(), name: favorite.name.clone() };
        self.sidebar_item(id, label, favorite.path.clone(), Some(row), cx)
            .when(!renaming, |item| {
                item.on_drag(drag, move |drag, _, _, cx| {
                    let _ = browser.update(cx, |browser, _| browser.reordering_favorite = true);
                    cx.new(|_| DragLabel(drag.name.clone()))
                })
            })
            .on_drag_move::<FavoriteDrag>(cx.listener(move |this, event: &DragMoveEvent<FavoriteDrag>, _, cx| {
                this.track_gap(GapOwner::Row(row), event.bounds, event.event.position, false, cx);
            }))
            .on_drop(cx.listener(|this, drag: &FavoriteDrag, _, cx| this.drop_favorite(drag, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.menu = Some(Menu::Favorite(row, event.position));
                cx.notify();
            }))
    }

    fn drop_favorite(&mut self, drag: &FavoriteDrag, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if let Some(gap) = self.take_gap() {
            let path = [drag.path.clone()];
            self.edit_favorites(|list| place_favorites(list, &path, gap), cx);
        }
    }

    /// 見出し。ドラッグを受けて、隣の行の間として働く。
    pub fn sidebar_heading(&self, label: &'static str, gap: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let owner = GapOwner::Heading(gap);
        div()
            .id(label)
            .px_2()
            .pt_3()
            .pb_1()
            .text_size(px(theme::SMALL_TEXT_SIZE))
            .text_color(rgb(theme::SUB_TEXT))
            .child(label)
            .on_drag_move::<ExternalPaths>(cx.listener(move |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                this.track_gap(owner, event.bounds, event.event.position, true, cx);
            }))
            .on_drag_move::<FavoriteDrag>(cx.listener(move |this, event: &DragMoveEvent<FavoriteDrag>, _, cx| {
                this.track_gap(owner, event.bounds, event.event.position, true, cx);
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                cx.stop_propagation();
                this.end_spring();
                if let Some(gap) = this.take_gap() {
                    this.register_dropped(paths.paths().to_vec(), gap, cx);
                }
            }))
            .on_drop(cx.listener(|this, drag: &FavoriteDrag, _, cx| this.drop_favorite(drag, cx)))
    }

    /// よく使う項目の行と、ドラッグの行き先の線。線は高さを持たず、行を動かさない。
    pub fn favorite_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let line = || div().relative().h(px(0.)).child(div().absolute().left_2().right_2().top(px(-1.)).h(px(2.)).bg(rgb(theme::SELECTED))).into_any_element();
        let gap = self.favorite_gap.map(|gap| gap.gap);
        let mut rows = Vec::new();
        for (row, favorite) in self.favorites.iter().enumerate() {
            if gap == Some(row) {
                rows.push(line());
            }
            rows.push(self.favorite_row(row, favorite, cx).into_any_element());
        }
        if gap == Some(self.favorites.len()) {
            rows.push(line());
        }
        rows
    }

    /// 行の右クリックのメニュー。
    pub fn favorite_menu(&self, row: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(favorite) = self.favorites.get(row) else { return Vec::new() };
        let (rename_path, remove_path) = (favorite.path.clone(), favorite.path.clone());
        vec![
            Self::menu_item("favorite-rename".into(), "名前を変更".into(), false)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.menu = None;
                    this.begin_favorite_rename(rename_path.clone(), window, cx);
                }))
                .into_any_element(),
            Self::menu_item("favorite-remove".into(), "よく使う項目から外す".into(), false)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.menu = None;
                    this.edit_favorites(|list| remove_favorite(list, &remove_path), cx);
                }))
                .into_any_element(),
        ]
    }

    fn begin_favorite_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.favorites.iter().find(|favorite| favorite.path == path).map(|favorite| favorite.name.clone()) else { return };
        let length = name.len();
        let input = cx.new(|cx| TextInput::new(name, 0..length, None, cx));
        let changed = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::Changed => {}
            InputEvent::Submit => this.end_favorite_rename(true, window, cx),
            InputEvent::Cancel => this.end_favorite_rename(false, window, cx),
        });
        let handle = input.focus_handle(cx);
        let blurred = cx.on_blur(&handle, window, |this, _, cx| this.finish_favorite_rename(true, cx));
        window.focus(&handle);
        self.favorite_rename = Some(FavoriteRename { path, input, _subscriptions: [changed, blurred] });
        cx.notify();
    }

    fn end_favorite_rename(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_favorite_rename(keep, cx);
        window.focus(&self.focus);
    }

    fn finish_favorite_rename(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(rename) = self.favorite_rename.take() else { return };
        filer_shell::remove_ime_caret();
        if keep {
            let name = rename.input.read(cx).text();
            self.edit_favorites(|list| rename_favorite(list, &rename.path, &name), cx);
        }
        cx.notify();
    }
}
