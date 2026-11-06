use std::ffi::c_void;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::Ime::{
    CANDIDATEFORM, CFS_CANDIDATEPOS, CFS_POINT, COMPOSITIONFORM, ImmGetContext, ImmReleaseContext,
    ImmSetCandidateWindow, ImmSetCompositionWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateCaret, DestroyCaret, SetCaretPos};

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
