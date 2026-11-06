use filer_core::Date;
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

/// FILETIMEを現地時刻の日付にする。変換できない時は1601年1月1日。
pub fn local_date(filetime: i64) -> Date {
    to_local(filetime).map_or(Date { year: 1601, month: 1, day: 1 }, to_date)
}

/// 今日の現地時刻の日付。
pub fn today() -> Date {
    to_date(unsafe { GetLocalTime() })
}

/// FILETIMEを現地時刻の `YYYY/MM/DD HH:MM` にする。変換できない時は空文字列。
pub fn format_time(filetime: i64) -> String {
    to_local(filetime).map_or(String::new(), |t| {
        format!("{}/{:02}/{:02} {:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute)
    })
}

fn to_date(t: SYSTEMTIME) -> Date {
    Date { year: i32::from(t.wYear), month: u32::from(t.wMonth), day: u32::from(t.wDay) }
}

fn to_local(filetime: i64) -> Option<SYSTEMTIME> {
    if filetime <= 0 {
        return None;
    }
    let file = FILETIME {
        dwLowDateTime: filetime as u32,
        dwHighDateTime: (filetime >> 32) as u32,
    };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        FileTimeToSystemTime(&file, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(local)
}
