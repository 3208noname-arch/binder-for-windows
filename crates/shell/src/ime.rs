use std::ffi::c_void;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_CANDIDATEPOS, CFS_POINT, COMPOSITIONFORM, ImmGetContext, ImmReleaseContext,
    ImmSetCandidateWindow, ImmSetCompositionWindow,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{CreateCaret, DestroyCaret, GetCursorPos, SetCaretPos};

/// IMEのモード表示と変換候補が文字カーソルの真下に出るよう、見えないキャレットと位置を渡す。
/// 座標はウィンドウの中のピクセル。
pub fn place_ime_caret(hwnd: isize, x: i32, y: i32, height: i32) {
    let hwnd = HWND(hwnd as *mut c_void);
    let below = POINT { x, y: y + height };
    unsafe {
        let _ = CreateCaret(hwnd, None, 2, height);
        let _ = SetCaretPos(x, y);
        let context = ImmGetContext(hwnd);
        let composition = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: below, rcArea: RECT::default() };
        let candidate =
            CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_CANDIDATEPOS, ptCurrentPos: below, rcArea: RECT::default() };
        let _ = ImmSetCompositionWindow(context, &composition);
        let _ = ImmSetCandidateWindow(context, &candidate);
        let _ = ImmReleaseContext(hwnd, context);
    }
}

/// 文字入力が終わった時に呼ぶ。
pub fn remove_ime_caret() {
    let _ = unsafe { DestroyCaret() };
}

/// ウィンドウの中の座標でカーソルの位置を読む。ウィンドウの外でも読める。
pub fn cursor_position(hwnd: isize) -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point).ok()?;
        ScreenToClient(HWND(hwnd as *mut c_void), &mut point).ok().ok()?;
    }
    Some((point.x, point.y))
}

/// Ctrlが押されているか。ドラッグの間はキーの通知が届かないので、ドロップの時はこちらで調べる。
pub fn control_down() -> bool {
    let state = unsafe { GetAsyncKeyState(i32::from(VK_CONTROL.0)) };
    state < 0
}

/// 左ボタンが押されているか。ウィンドウの外で離しても分かる。
pub fn left_button_down() -> bool {
    let state = unsafe { GetAsyncKeyState(i32::from(VK_LBUTTON.0)) };
    state < 0
}
