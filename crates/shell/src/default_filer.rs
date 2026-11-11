//! フォルダを開く既定のアプリを、エクスプローラーとBinderの間で切り替える。
//!
//! 書く先はHKCUだけ。オンにする前に、触るキーを丸ごと `.reg` に書き出し、戻す時はその内容をそのまま入れ直す。
//! 書き出しの頭にキーを消す行を置くので、取り込むと書き出した時と同じ形に戻る。ファイルをダブルクリックしても戻せる。

use std::ffi::OsStr;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use windows::core::PCWSTR;

const FOLDER_SHELL: &str = r"Software\Classes\Folder\shell";
const WIN_E: &str = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
const OPEN: &str = r"Software\Classes\Folder\shell\open\command";
const EXPLORE: &str = r"Software\Classes\Folder\shell\explore\command";
const WIN_E_COMMAND: &str = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command";
/// 書き出して戻すキー。値を書く場所はすべてこの下にある。
const TOUCHED: [&str; 2] = [FOLDER_SHELL, WIN_E];
const REG_HEADER: &str = "Windows Registry Editor Version 5.00";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 今の切り替えの状態。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultFiler {
    /// Binderを指す値が1つも無い。
    Explorer,
    /// 3か所すべてがこのexeを指す。
    Binder,
    /// 3か所すべてがBinderを指すが、別の場所のexeである。
    OtherBinder(String),
    /// 一部だけがBinderを指す。
    Partial,
    /// 3か所はBinderを指すが、他のアプリ（QTTabBarなど）がフォルダの既定の動作を `open` 以外に変えている。
    VerbChanged(String),
}

/// フォルダの既定の動作が `open` か。空ならHKLMの既定が効き、それも `open` である。
fn verb_is_open(verb: Option<&str>) -> bool {
    let first = verb.unwrap_or_default().split(',').next().unwrap_or_default().trim();
    first.is_empty() || first.eq_ignore_ascii_case("open")
}

/// フォルダを開く時のコマンド。
pub fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Win+Eのコマンド。フォルダを渡さないので、ホームで開く。
pub fn win_e_command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

fn is_binder(command: &str) -> bool {
    command.to_ascii_lowercase().contains("binder.exe")
}

/// 3か所のコマンドとフォルダの既定の動作から状態を決める。
pub fn classify(open: Option<&str>, explore: Option<&str>, win_e: Option<&str>, verb: Option<&str>, exe: &Path) -> DefaultFiler {
    let values = [open, explore, win_e];
    let binder = values.iter().filter(|value| value.is_some_and(is_binder)).count();
    if binder == 0 {
        return DefaultFiler::Explorer;
    }
    if binder < values.len() {
        return DefaultFiler::Partial;
    }
    if !verb_is_open(verb) {
        return DefaultFiler::VerbChanged(verb.unwrap_or_default().to_string());
    }
    let ours = open == Some(open_command(exe).as_str()) && explore == Some(open_command(exe).as_str()) && win_e == Some(win_e_command(exe).as_str());
    if ours { DefaultFiler::Binder } else { DefaultFiler::OtherBinder(open.unwrap_or_default().to_string()) }
}

/// 今の状態を読む。
pub fn default_filer_state(exe: &Path) -> DefaultFiler {
    classify(read_default(OPEN).as_deref(), read_default(EXPLORE).as_deref(), read_default(WIN_E_COMMAND).as_deref(), read_default(FOLDER_SHELL).as_deref(), exe)
}

/// オンにする。書き出しが無い時だけ、書き込む前に今の内容を書き出す。
/// QTTabBarのように、フォルダの既定の動作を別の動作に変えるアプリがある。その時は `open` が呼ばれないので、既定を `open` に戻す。
pub fn set_binder_default(exe: &Path, backup: &Path) -> io::Result<()> {
    if !backup.exists() {
        write_backup(backup)?;
    }
    if !verb_is_open(read_default(FOLDER_SHELL).as_deref()) {
        set_value(FOLDER_SHELL, None, REG_SZ, "open")?;
    }
    let open = open_command(exe);
    set_value(OPEN, None, REG_EXPAND_SZ, &open)?;
    set_value(OPEN, Some("DelegateExecute"), REG_SZ, "")?;
    set_value(EXPLORE, None, REG_EXPAND_SZ, &open)?;
    set_value(EXPLORE, Some("DelegateExecute"), REG_SZ, "")?;
    set_value(WIN_E_COMMAND, None, REG_EXPAND_SZ, &win_e_command(exe))?;
    set_value(WIN_E_COMMAND, Some("DelegateExecute"), REG_SZ, "")?;
    notify();
    Ok(())
}

/// 戻した後に書き出しを残しておく名前。次にオンにした時の書き出しと取り違えないよう、名前を変える。
pub fn restored_backup(backup: &Path) -> PathBuf {
    backup.with_extension("restored.reg")
}

/// 既定がBinderでない時に、使わなかった書き出しを残しておく名前。同じ名前があれば番号を付けて、前のものを上書きしない。
pub fn unused_backup(backup: &Path) -> PathBuf {
    let first = backup.with_extension("unused.reg");
    if !first.exists() {
        return first;
    }
    (2..).map(|number| backup.with_extension(format!("unused-{number}.reg"))).find(|path| !path.exists()).unwrap_or(first)
}

/// どう戻したか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restored {
    /// 書き出した内容へ正確に戻した。
    FromBackup,
    /// 書き出しが無いので、Binderを指すキーだけを消した。HKLMのエクスプローラーの値が効く。
    RemovedOnly,
    /// 既定がBinderを指していないので、書き出しを使わず、名前を変えて残した。レジストリには触っていない。
    BackupSetAside,
}

/// 戻す時に、書き出しをどう扱うか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestorePlan {
    /// 書き出しを取り込む。
    Import,
    /// 書き出しを取り込まずに脇へ置く。古い書き出しで、今のエクスプローラーなどの設定を上書きしないため。
    SetAside,
    /// Binderを指すキーだけを消す。
    RemoveBinderKeys,
}

fn restore_plan(points_to_binder: bool, backup_exists: bool) -> RestorePlan {
    match (backup_exists, points_to_binder) {
        (true, true) => RestorePlan::Import,
        (true, false) => RestorePlan::SetAside,
        (false, _) => RestorePlan::RemoveBinderKeys,
    }
}

/// 3か所のどれかがBinderを指すか。
fn points_to_binder() -> bool {
    [OPEN, EXPLORE, WIN_E_COMMAND].iter().any(|key| read_default(key).is_some_and(|command| is_binder(&command)))
}

/// 既定がBinderを指す時だけ、書き出した内容へ戻す。書き出しが無い時は、Binderを指すキーだけを消す。
/// Binderを指していない時は、書き出しが古いかもしれないので取り込まず、名前を変えて残す。
pub fn restore_explorer(backup: &Path) -> io::Result<Restored> {
    match restore_plan(points_to_binder(), backup.exists()) {
        RestorePlan::Import => {
            run_reg(&[OsStr::new("import"), backup.as_os_str()])?;
            std::fs::rename(backup, restored_backup(backup))?;
            notify();
            Ok(Restored::FromBackup)
        }
        RestorePlan::SetAside => {
            std::fs::rename(backup, unused_backup(backup))?;
            Ok(Restored::BackupSetAside)
        }
        RestorePlan::RemoveBinderKeys => {
            for (value_key, remove) in [(OPEN, OPEN), (EXPLORE, EXPLORE), (WIN_E_COMMAND, WIN_E)] {
                if read_default(value_key).is_some_and(|command| is_binder(&command)) {
                    delete_tree(remove)?;
                }
            }
            notify();
            Ok(Restored::RemovedOnly)
        }
    }
}

fn delete_tree(key: &str) -> io::Result<()> {
    let key = wide(key);
    let done = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr())) };
    if done == ERROR_SUCCESS || done.0 == 2 { Ok(()) } else { Err(io::Error::from_raw_os_error(done.0 as i32)) }
}

/// 触るキーを1つの `.reg` に書き出す。
fn write_backup(backup: &Path) -> io::Result<()> {
    let folder = backup.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(folder)?;
    let mut exports = Vec::new();
    for (index, key) in TOUCHED.iter().enumerate() {
        let part = folder.join(format!("export-{index}.reg"));
        let full = format!(r"HKCU\{key}");
        let done = run_reg(&[OsStr::new("export"), OsStr::new(&full), part.as_os_str(), OsStr::new("/y")]);
        // 無いキーは書き出せない。消す行だけで、無い状態に戻る
        exports.push(if done.is_ok() { decode_utf16(&std::fs::read(&part)?) } else { String::new() });
        let _ = std::fs::remove_file(&part);
    }
    let text = backup_text(&TOUCHED, &exports);
    std::fs::write(backup, encode_utf16(&text))
}

/// 消す行と書き出しの本文をつないだ `.reg` の中身。
pub fn backup_text(keys: &[&str], exports: &[String]) -> String {
    let mut text = format!("{REG_HEADER}\r\n\r\n");
    for key in keys {
        text.push_str(&format!("[-HKEY_CURRENT_USER\\{key}]\r\n\r\n"));
    }
    for export in exports {
        let body = export.trim_start_matches('\u{feff}').trim_start();
        let body = body.strip_prefix(REG_HEADER).unwrap_or(body).trim();
        if !body.is_empty() {
            text.push_str(body);
            text.push_str("\r\n\r\n");
        }
    }
    text
}

fn decode_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
    String::from_utf16_lossy(&units)
}

fn encode_utf16(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    bytes
}

fn run_reg(args: &[&OsStr]) -> io::Result<()> {
    let reg = std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join(r"System32\reg.exe")).unwrap_or_else(|| "reg.exe".into());
    let output = Command::new(reg).args(args).creation_flags(CREATE_NO_WINDOW).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("reg.exe が失敗しました（{}）", output.status)))
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn read_default(key: &str) -> Option<String> {
    let key = wide(key);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut size = 0u32;
    let mut kind = REG_VALUE_TYPE::default();
    unsafe {
        if RegGetValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR::null(), flags, Some(&mut kind), None, Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = vec![0u16; size as usize / 2 + 1];
        if RegGetValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR::null(), flags, Some(&mut kind), Some(buffer.as_mut_ptr().cast()), Some(&mut size)) != ERROR_SUCCESS {
            return None;
        }
        let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end]))
    }
}

fn set_value(key: &str, name: Option<&str>, kind: REG_VALUE_TYPE, data: &str) -> io::Result<()> {
    let key = wide(key);
    let name = name.map(wide);
    let bytes: Vec<u8> = wide(data).into_iter().flat_map(u16::to_le_bytes).collect();
    unsafe {
        let mut handle = HKEY::default();
        let created = RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, None, &mut handle, None);
        if created != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(created.0 as i32));
        }
        let name = name.as_ref().map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
        let set = RegSetValueExW(handle, name, None, kind, Some(&bytes));
        let _ = RegCloseKey(handle);
        if set != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(set.0 as i32));
        }
    }
    Ok(())
}

fn notify() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}

/// 起動の引数をパスとして読める形に直す。`"C:\"` は引数の解釈で `C:"` になって届く。
pub fn clean_argument(argument: &str) -> String {
    let trimmed = argument.trim().trim_end_matches('"');
    if trimmed.len() == 2 && trimmed.ends_with(':') { format!("{trimmed}\\") } else { trimmed.to_string() }
}

/// エクスプローラーへ渡したことを覚えるファイルに書く1行。
fn forward_record(argument: &str, now_ms: u128) -> String {
    format!("{now_ms}\t{argument}")
}

/// 同じ引数を直前に渡していれば、渡し直さない。エクスプローラーがBinderを呼び返した時に、起動が繰り返されないようにする。
pub fn should_forward(record: Option<&str>, argument: &str, now_ms: u128, window_ms: u128) -> bool {
    let Some((time, previous)) = record.and_then(|line| line.split_once('\t')) else { return true };
    let Ok(time) = time.parse::<u128>() else { return true };
    !(previous == argument && now_ms.saturating_sub(time) < window_ms)
}

/// 実在しないフォルダ（ごみ箱や `shell:` など）をエクスプローラーへ渡す。渡したら `true`。
pub fn forward_to_explorer(argument: &OsStr, record_file: &Path) -> bool {
    let text = argument.to_string_lossy().to_string();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_millis()).unwrap_or(0);
    let record = std::fs::read_to_string(record_file).ok();
    if !should_forward(record.as_deref(), &text, now, 5000) {
        return false;
    }
    if let Some(folder) = record_file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let _ = std::fs::write(record_file, forward_record(&text, now));
    let explorer = std::env::var_os("SystemRoot").map(|root| PathBuf::from(root).join("explorer.exe")).unwrap_or_else(|| "explorer.exe".into());
    Command::new(explorer).arg(argument).spawn().is_ok()
}

/// 引数をBinderで開けるか。`::{CLSID}`、`shell:`、サーバー名だけのパスなどは、実在しないので開けない。
pub fn can_open(argument: &OsStr) -> bool {
    Path::new(argument).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe() -> PathBuf {
        PathBuf::from(r"C:\Apps\Binder\binder.exe")
    }

    #[test]
    fn commands_quote_the_exe_and_the_folder() {
        assert_eq!(open_command(&exe()), r#""C:\Apps\Binder\binder.exe" "%1""#);
        assert_eq!(win_e_command(&exe()), r#""C:\Apps\Binder\binder.exe""#);
    }

    #[test]
    fn classify_tells_explorer_binder_other_and_partial() {
        let open = open_command(&exe());
        let win_e = win_e_command(&exe());
        assert_eq!(classify(None, None, None, None, &exe()), DefaultFiler::Explorer);
        assert_eq!(classify(Some(r"%SystemRoot%\Explorer.exe"), None, None, None, &exe()), DefaultFiler::Explorer);
        assert_eq!(classify(Some(&open), Some(&open), Some(&win_e), None, &exe()), DefaultFiler::Binder);
        assert_eq!(classify(Some(&open), Some(&open), Some(&win_e), Some("open"), &exe()), DefaultFiler::Binder);
        assert_eq!(classify(Some(&open), None, Some(&win_e), None, &exe()), DefaultFiler::Partial);
        let other = open_command(Path::new(r"D:\old\binder.exe"));
        let other_e = win_e_command(Path::new(r"D:\old\binder.exe"));
        assert_eq!(classify(Some(&other), Some(&other), Some(&other_e), None, &exe()), DefaultFiler::OtherBinder(other.clone()));
    }

    #[test]
    fn a_changed_folder_verb_is_reported() {
        let open = open_command(&exe());
        let win_e = win_e_command(&exe());
        let qt = "QTTabBar.openInTab, QTTabBar.openNewTab, opennewwindow";
        assert_eq!(classify(Some(&open), Some(&open), Some(&win_e), Some(qt), &exe()), DefaultFiler::VerbChanged(qt.to_string()));
        assert!(verb_is_open(Some("open, opennewwindow")));
        assert!(!verb_is_open(Some("opennewwindow")));
        assert!(verb_is_open(Some("")));
    }

    #[test]
    fn backup_deletes_every_key_then_restores_the_exports() {
        let export = format!("\u{feff}{REG_HEADER}\r\n\r\n[HKEY_CURRENT_USER\\A]\r\n@=\"x\"\r\n\r\n");
        let text = backup_text(&["A", "B"], &[export, String::new()]);
        assert_eq!(text, format!("{REG_HEADER}\r\n\r\n[-HKEY_CURRENT_USER\\A]\r\n\r\n[-HKEY_CURRENT_USER\\B]\r\n\r\n[HKEY_CURRENT_USER\\A]\r\n@=\"x\"\r\n\r\n"));
    }

    #[test]
    fn the_backup_is_imported_only_while_binder_is_the_default() {
        assert_eq!(restore_plan(true, true), RestorePlan::Import);
        assert_eq!(restore_plan(false, true), RestorePlan::SetAside);
        assert_eq!(restore_plan(true, false), RestorePlan::RemoveBinderKeys);
        assert_eq!(restore_plan(false, false), RestorePlan::RemoveBinderKeys);
    }

    #[test]
    fn an_unused_backup_never_replaces_an_earlier_one() {
        let folder = std::env::temp_dir().join(format!("binder-unused-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let backup = folder.join("explorer-backup.reg");
        assert_eq!(unused_backup(&backup), folder.join("explorer-backup.unused.reg"));
        assert_ne!(unused_backup(&backup), restored_backup(&backup));
        std::fs::write(folder.join("explorer-backup.unused.reg"), "").unwrap();
        assert_eq!(unused_backup(&backup), folder.join("explorer-backup.unused-2.reg"));
        std::fs::write(folder.join("explorer-backup.unused-2.reg"), "").unwrap();
        assert_eq!(unused_backup(&backup), folder.join("explorer-backup.unused-3.reg"));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn utf16_round_trips_with_a_bom() {
        let bytes = encode_utf16("日本 reg");
        assert_eq!(&bytes[..2], &[0xff, 0xfe]);
        assert_eq!(decode_utf16(&bytes).trim_start_matches('\u{feff}'), "日本 reg");
    }

    #[test]
    fn clean_argument_fixes_a_quoted_drive_root() {
        assert_eq!(clean_argument("C:\""), r"C:\");
        assert_eq!(clean_argument(r"C:\Users"), r"C:\Users");
        assert_eq!(clean_argument("::{645FF040-5081-101B-9F08-00AA002F954E}"), "::{645FF040-5081-101B-9F08-00AA002F954E}");
    }

    #[test]
    fn real_folders_are_opened_and_virtual_ones_are_not() {
        let folder = std::env::temp_dir();
        assert!(can_open(folder.as_os_str()));
        assert!(!can_open(OsStr::new("::{645FF040-5081-101B-9F08-00AA002F954E}")));
        assert!(!can_open(OsStr::new("shell:RecycleBinFolder")));
        assert!(!can_open(OsStr::new("::{26EE0668-A00A-44D7-9371-BEB064C98683}\0")));
    }

    #[test]
    fn the_same_argument_is_not_forwarded_twice_in_a_row() {
        let record = forward_record("::{X}", 10_000);
        assert!(!should_forward(Some(&record), "::{X}", 12_000, 5000));
        assert!(should_forward(Some(&record), "::{X}", 16_000, 5000));
        assert!(should_forward(Some(&record), "::{Y}", 12_000, 5000));
        assert!(should_forward(None, "::{X}", 12_000, 5000));
        assert!(should_forward(Some("broken"), "::{X}", 12_000, 5000));
    }
}
