use std::cell::RefCell;
use std::cmp::Ordering;
use windows::Win32::UI::Shell::StrCmpLogicalW;
use windows::core::PCWSTR;

thread_local! {
    // 並べ替えでは比較が数百万回になるので、変換用の領域を使い回す
    static BUFFERS: RefCell<(Vec<u16>, Vec<u16>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

fn fill(buffer: &mut Vec<u16>, text: &str) {
    buffer.clear();
    buffer.extend(text.encode_utf16());
    buffer.push(0);
}

/// エクスプローラーと同じ名前の順で比べる。
pub fn compare_names(a: &str, b: &str) -> Ordering {
    BUFFERS.with_borrow_mut(|(left, right)| {
        fill(left, a);
        fill(right, b);
        unsafe { StrCmpLogicalW(PCWSTR(left.as_ptr()), PCWSTR(right.as_ptr())) }.cmp(&0)
    })
}
