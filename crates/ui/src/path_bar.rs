//! ステータスバーのパス欄。普段は今のフォルダを表示し、クリックかCtrl+Lで入力欄になる。

use crate::browser::Browser;
use crate::text_input::{InputEvent, TextInput};
use crate::theme;
use filer_core::parse_path_input;
use filer_shell::{Probed, probe_path, remove_ime_caret};
use gpui::{AnyElement, Context, CursorStyle, Entity, Focusable, Subscription, Window, div, prelude::*, px, rgb};
use std::path::PathBuf;

pub struct PathEdit {
    input: Entity<TextInput>,
    /// パスの先を確かめている間は真
    checking: bool,
    _subscriptions: [Subscription; 2],
}

impl Browser {
    /// 入力欄を開いて全体を選ぶ。そのままコピーも上書きもできる。
    pub fn open_path_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_rename(window, cx);
        if let Some(edit) = &self.path_edit {
            window.focus(&edit.input.focus_handle(cx));
            return;
        }
        let text = self.current_folder(cx).display().to_string();
        let length = text.len();
        let input = cx.new(|cx| TextInput::new(text, 0..length, None, cx));
        let changed = cx.subscribe_in(&input, window, |this, _, event, _, cx| match event {
            InputEvent::Changed => {
                this.tab.message = None;
                cx.notify();
            }
            InputEvent::Submit => this.submit_path(cx),
            InputEvent::Cancel => {
                this.tab.message = None;
                this.close_path_edit(true, cx);
            }
        });
        let handle = input.focus_handle(cx);
        let blurred = cx.on_blur(&handle, window, |this, _, cx| this.close_path_edit(false, cx));
        window.focus(&handle);
        self.path_edit = Some(PathEdit { input, checking: false, _subscriptions: [changed, blurred] });
        cx.notify();
    }

    /// 入力欄を閉じる。確かめている途中の結果は捨てる。
    pub fn close_path_edit(&mut self, refocus: bool, cx: &mut Context<Self>) {
        if self.path_edit.take().is_none() {
            return;
        }
        self.path_request += 1;
        remove_ime_caret();
        self.refocus |= refocus;
        cx.notify();
    }

    /// 文字を入力中か。パス欄の中のキーを、一覧の操作へ渡さない。
    pub fn editing_path(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.path_edit.as_ref().is_some_and(|edit| edit.input.focus_handle(cx).is_focused(window))
    }

    fn submit_path(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = &mut self.path_edit else { return };
        let text = edit.input.read(cx).text();
        let current = self.tab.folder(cx);
        let Some(path) = parse_path_input(&text, &current, |name| std::env::var(name).ok()) else {
            self.close_path_edit(true, cx);
            return;
        };
        edit.checking = true;
        self.path_request += 1;
        let request = self.path_request;
        cx.spawn(async move |this, cx| {
            let target = path.clone();
            let found = cx.background_spawn(async move { probe_path(&target) }).await;
            let _ = this.update(cx, |browser, cx| browser.finish_path(request, path, found, cx));
        })
        .detach();
        cx.notify();
    }

    fn finish_path(&mut self, request: u64, typed: PathBuf, found: Probed, cx: &mut Context<Self>) {
        if request != self.path_request {
            return;
        }
        let current = self.current_folder(cx);
        match found {
            Probed::Folder(path) => {
                self.close_path_edit(true, cx);
                if path != current {
                    self.navigate(path, cx);
                }
            }
            Probed::File(path) => {
                self.close_path_edit(true, cx);
                let Some(parent) = path.parent().map(PathBuf::from) else { return };
                if parent != current {
                    self.navigate(parent, cx);
                }
                self.tab.pending_select = Some(path);
                self.tab.synced_version = 0;
            }
            Probed::Missing => self.reject_path(format!("「{}」が見つかりません", typed.display()), cx),
            Probed::Denied => self.reject_path(format!("「{}」にアクセスできません", typed.display()), cx),
        }
    }

    /// 打ち直せるよう、入力欄は開いたままにする。
    fn reject_path(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(edit) = &mut self.path_edit {
            edit.checking = false;
        }
        self.show_message(message, cx);
    }

    /// 確かめている間に出す文字。
    pub fn path_status(&self) -> Option<&'static str> {
        self.path_edit.as_ref().is_some_and(|edit| edit.checking).then_some("確認中…")
    }

    pub fn path_field(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(edit) = &self.path_edit {
            return div()
                .flex_1()
                .min_w_0()
                .px_1()
                .rounded_sm()
                .overflow_hidden()
                .whitespace_nowrap()
                .bg(rgb(theme::BACKGROUND))
                .border_1()
                .border_color(rgb(theme::SELECTED))
                .text_color(rgb(theme::TEXT))
                .child(edit.input.clone())
                .into_any_element();
        }
        div()
            .id("path-field")
            .flex_1()
            .min_w_0()
            .h(px(theme::STATUS_HEIGHT))
            .line_height(px(theme::STATUS_HEIGHT))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_right()
            .cursor(CursorStyle::IBeam)
            .child(self.current_folder(cx).display().to_string())
            .on_click(cx.listener(|this, _, window, cx| this.open_path_edit(window, cx)))
            .into_any_element()
    }
}
