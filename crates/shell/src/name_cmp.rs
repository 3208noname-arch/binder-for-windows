use std::cmp::Ordering;
use windows::Win32::UI::Shell::StrCmpLogicalW;
use windows::core::PCWSTR;

/// エクスプローラーと同じ名前の順で比べる。
pub fn compare_names(a: &str, b: &str) -> Ordering {
    let a: Vec<u16> = a.encode_utf16().chain([0]).collect();
    let b: Vec<u16> = b.encode_utf16().chain([0]).collect();
    unsafe { StrCmpLogicalW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr())) }.cmp(&0)
}
