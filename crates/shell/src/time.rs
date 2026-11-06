use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

/// FILETIMEを現地時刻の `YYYY/MM/DD HH:MM` にする。変換できない時は空文字列。
pub fn format_time(filetime: i64) -> String {
    if filetime <= 0 {
        return String::new();
    }
    let file = FILETIME {
        dwLowDateTime: filetime as u32,
        dwHighDateTime: (filetime >> 32) as u32,
    };
    let (mut utc, mut t) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    let converted = unsafe {
        FileTimeToSystemTime(&file, &mut utc)
            .and_then(|()| SystemTimeToTzSpecificLocalTime(None, &utc, &mut t))
    };
    match converted {
        Ok(()) => format!("{}/{:02}/{:02} {:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute),
        Err(_) => String::new(),
    }
}
