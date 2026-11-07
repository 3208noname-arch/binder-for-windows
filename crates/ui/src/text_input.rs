//! 名前の変更に使う入力欄。gpuiの入力欄のサンプル（Apache-2.0）を元にしている。

use filer_shell::place_ime_caret;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, SharedString, Style, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window, WrappedLine, actions, div, fill, point, prelude::*, px, relative, rgb, rgba, size, white,
};
use std::ops::Range;
use unicode_segmentation::*;
use crate::theme;

actions!(
    text_input,
    [Backspace, Delete, Left, Right, SelectLeft, SelectRight, SelectAll, Home, End, Paste, Cut, Copy, Submit, Cancel]
);

/// 入力欄の結果。
pub enum InputEvent {
    /// 文字が確定して変わった
    Changed,
    Submit,
    Cancel,
}

impl EventEmitter<InputEvent> for TextInput {}

/// 入力欄のキー操作を登録する。起動の時に1回呼ぶ。
pub fn bind_keys(cx: &mut App) {
    const CONTEXT: Option<&str> = Some("TextInput");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, CONTEXT),
        KeyBinding::new("delete", Delete, CONTEXT),
        KeyBinding::new("left", Left, CONTEXT),
        KeyBinding::new("right", Right, CONTEXT),
        KeyBinding::new("shift-left", SelectLeft, CONTEXT),
        KeyBinding::new("shift-right", SelectRight, CONTEXT),
        KeyBinding::new("ctrl-a", SelectAll, CONTEXT),
        KeyBinding::new("ctrl-v", Paste, CONTEXT),
        KeyBinding::new("ctrl-c", Copy, CONTEXT),
        KeyBinding::new("ctrl-x", Cut, CONTEXT),
        KeyBinding::new("home", Home, CONTEXT),
        KeyBinding::new("end", End, CONTEXT),
        KeyBinding::new("enter", Submit, CONTEXT),
        KeyBinding::new("escape", Cancel, CONTEXT),
    ]);
}

/// 折り返した文字の、見た目の1行。
struct VisualLine {
    range: Range<usize>,
    /// 折り返す前の1行の中での、この行の始まり
    start_x: Pixels,
    /// 欄の左端からの、この行の始まり
    left: Pixels,
}

/// 文字の並びと、文字の番号と画面の位置の対応。折り返しの有無によらず同じ形で扱う。
struct TextLayout {
    shaped: WrappedLine,
    lines: Vec<VisualLine>,
    line_height: Pixels,
}

impl TextLayout {
    /// `center` は中央揃えにする欄の幅。
    fn new(shaped: WrappedLine, center: Option<Pixels>, line_height: Pixels) -> Self {
        let line = &shaped.unwrapped_layout;
        let mut marks: Vec<(usize, Pixels)> = vec![(0, px(0.))];
        for boundary in &shaped.wrap_boundaries {
            let glyph = &line.runs[boundary.run_ix].glyphs[boundary.glyph_ix];
            marks.push((glyph.index, glyph.position.x));
        }
        marks.push((shaped.len(), line.width));
        let lines = marks
            .windows(2)
            .map(|pair| {
                let ((start, start_x), (end, end_x)) = (pair[0], pair[1]);
                let left = center.map_or(px(0.), |width| (width - (end_x - start_x)) / 2.);
                VisualLine { range: start..end, start_x, left }
            })
            .collect();
        Self { shaped, lines, line_height }
    }

    fn line_of(&self, index: usize) -> usize {
        self.lines.iter().rposition(|line| line.range.start <= index).unwrap_or(0)
    }

    /// 文字の番号の位置。欄の左上からの距離。
    fn position(&self, index: usize) -> Point<Pixels> {
        let row = self.line_of(index);
        let line = &self.lines[row];
        let x = self.shaped.unwrapped_layout.x_for_index(index) - line.start_x + line.left;
        point(x, self.line_height * row as f32)
    }

    /// 欄の左上からの距離に、いちばん近い文字の番号。
    fn index_at(&self, position: Point<Pixels>) -> usize {
        let row = (f32::from(position.y / self.line_height).max(0.) as usize).min(self.lines.len() - 1);
        let line = &self.lines[row];
        let index = self.shaped.unwrapped_layout.closest_index_for_x(position.x - line.left + line.start_x);
        index.clamp(line.range.start, line.range.end)
    }

    /// 範囲を見た目の行ごとに切った、欄の左上からの四角。
    fn rectangles(&self, range: &Range<usize>) -> Vec<Bounds<Pixels>> {
        self.lines
            .iter()
            .filter(|line| line.range.start < range.end && range.start < line.range.end)
            .map(|line| {
                let (start, end) = (range.start.max(line.range.start), range.end.min(line.range.end));
                let top = self.lines.iter().position(|other| other.range == line.range).unwrap_or(0) as f32;
                let unwrapped = &self.shaped.unwrapped_layout;
                let x = |index| unwrapped.x_for_index(index) - line.start_x + line.left;
                Bounds::new(point(x(start), self.line_height * top), size(x(end) - x(start), self.line_height))
            })
            .collect()
    }
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<TextLayout>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    /// 長い名前の時に、文字カーソルが見えるよう左へずらす量
    shift: Pixels,
    /// 折り返す幅。無ければ1行で横に送る
    wrap: Option<f32>,
}

impl TextInput {
    /// `selected` の範囲を選んだ状態で作る。範囲はバイト単位。`wrap` は折り返す幅。
    pub fn new(text: String, selected: Range<usize>, wrap: Option<f32>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: text.into(),
            selected_range: selected,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            shift: px(0.),
            wrap,
        }
    }

    pub fn text(&self) -> String {
        self.content.to_string()
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Cancel);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace("\n", " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.index_at(point(position.x - bounds.left() + self.shift, position.y - bounds.top()))
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.emit(InputEvent::Changed);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let (start, end) = (layout.position(range.start), layout.position(range.end));
        Some(Bounds::from_corners(
            point(bounds.left() + start.x, bounds.top() + start.y),
            point(bounds.left() + end.x, bounds.top() + end.y + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.last_bounds?.localize(&point)?;
        let utf8_index = self.last_layout.as_ref()?.index_at(local);
        Some(self.offset_to_utf16(utf8_index))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    layout: Option<TextLayout>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    shift: Pixels,
}

/// 入力欄の文字を組む。
fn shape(input: &TextInput, window: &mut Window) -> Option<WrappedLine> {
    let style = window.text_style();
    let content = input.content.clone();
    let run = TextRun {
        len: content.len(),
        font: style.font(),
        color: style.color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let runs: Vec<TextRun> = if let Some(marked_range) = input.marked_range.as_ref() {
        vec![
            TextRun { len: marked_range.start, ..run.clone() },
            TextRun {
                len: marked_range.end - marked_range.start,
                underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.0), wavy: false }),
                ..run.clone()
            },
            TextRun { len: content.len() - marked_range.end, ..run },
        ]
        .into_iter()
        .filter(|run| run.len > 0)
        .collect()
    } else {
        vec![run]
    };
    let font_size = style.font_size.to_pixels(window.rem_size());
    let wrap = input.wrap.map(px);
    window.text_system().shape_text(content, font_size, &runs, wrap, None).ok()?.into_iter().next()
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let line_height = window.line_height();
        let rows = shape(self.input.read(cx), window).map_or(1, |line| line.wrap_boundaries.len() + 1);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (line_height * rows as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let line_height = window.line_height();
        let center = input.wrap.map(|_| bounds.size.width);
        let layout = shape(input, window).map(|line| TextLayout::new(line, center, line_height));

        let at = |index: usize| layout.as_ref().map_or(point(px(0.), px(0.)), |layout| layout.position(index));
        let cursor_at = at(cursor);
        let shift = if center.is_some() { px(0.) } else { (cursor_at.x + px(2.) - bounds.size.width).max(px(0.)) };
        let origin = point(bounds.left() - shift, bounds.top());
        let (selection, cursor) = if selected_range.is_empty() {
            let caret = Bounds::new(origin + cursor_at, size(px(2.), line_height));
            (Vec::new(), Some(fill(caret, gpui::blue())))
        } else {
            let parts = layout.as_ref().map_or(Vec::new(), |layout| layout.rectangles(&selected_range));
            let quads = parts.into_iter().map(|part| fill(Bounds::new(origin + part.origin, part.size), rgba(0x3311ff30))).collect();
            (quads, None)
        };
        PrepaintState { layout, cursor, selection, shift }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        let wrapped = self.input.read(cx).wrap.is_some();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let shift = prepaint.shift;
        // 欄からはみ出た文字は描かない
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for selection in std::mem::take(&mut prepaint.selection) {
                window.paint_quad(selection)
            }
            let origin = point(bounds.left() - shift, bounds.top());
            let align = if wrapped { TextAlign::Center } else { TextAlign::Left };
            if let Some(layout) = &prepaint.layout {
                layout.shaped.paint(origin, window.line_height(), align, Some(bounds), window, cx).unwrap();
            }
        });
        let layout = prepaint.layout.take();

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            // IMEのモード表示が文字カーソルの真下に出るよう、位置を渡し続ける
            let (b, scale) = (cursor.bounds, window.scale_factor());
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let RawWindowHandle::Win32(handle) = window.window_handle().unwrap().as_raw() {
                let to_px = |value: Pixels| (f32::from(value) * scale) as i32;
                place_ime_caret(handle.hwnd.get(), to_px(b.origin.x), to_px(b.origin.y), to_px(b.size.height));
            }
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = layout;
            input.last_bounds = Some(bounds);
            input.shift = shift;
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (text_size, line_height) = match self.wrap {
            Some(_) => (theme::SMALL_TEXT_SIZE, theme::GRID_LABEL_LINE),
            None => (13., 18.),
        };
        div()
            .flex()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .line_height(px(line_height))
            .text_size(px(text_size))
            .child(
                div()
                    .when(self.wrap.is_none(), |field| field.h(px(20.)))
                    .w_full()
                    .px(px(3.)).py(px(1.)).border_1().border_color(rgb(0x0a64d8)).rounded_sm().text_color(rgb(0x1f2023))
                    .bg(white())
                    .child(TextElement { input: cx.entity() }),
            )
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

