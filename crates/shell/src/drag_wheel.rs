//! ドラッグ中のホイール。ドラッグの間はウィンドウへ通知が届かないので、低レベルのフックで受け取る。

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, MSLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEWHEEL,
};

static WHEEL: AtomicI32 = AtomicI32::new(0);
static HOOK: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn note_wheel(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 && message.0 as u32 == WM_MOUSEWHEEL {
        let info = unsafe { &*(data.0 as *const MSLLHOOKSTRUCT) };
        let delta = (info.mouseData >> 16) as u16 as i16;
        WHEEL.fetch_add(i32::from(delta), Ordering::Relaxed);
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

/// ドラッグの間だけホイールを受け取る。何度呼んでもよい。メッセージを回すスレッドから呼ぶ。
pub fn watch_wheel(on: bool) {
    let hook = HOOK.load(Ordering::Relaxed);
    if on && hook == 0 {
        WHEEL.store(0, Ordering::Relaxed);
        if let Ok(hook) = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(note_wheel), None, 0) } {
            HOOK.store(hook.0 as isize, Ordering::Relaxed);
        }
    } else if !on && hook != 0 {
        let _ = unsafe { UnhookWindowsHookEx(HHOOK(hook as *mut _)) };
        HOOK.store(0, Ordering::Relaxed);
    }
}

/// 前回からのホイールの回転量。120で1目盛り。
pub fn take_wheel() -> i32 {
    WHEEL.swap(0, Ordering::Relaxed)
}
