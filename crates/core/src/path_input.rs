//! パス欄に打たれた文字を、開く先のパスへ直す。

use std::path::{Component, Path, PathBuf};

/// `%NAME%` を環境変数の値へ置き換える。知らない名前はそのまま残す。
fn expand_variables(text: &str, env: &impl Fn(&str) -> Option<String>) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let (before, after) = rest.split_at(start);
        result.push_str(before);
        let name = &after[1..];
        let found = name.find('%').and_then(|end| Some((env(&name[..end])?, end)));
        match found {
            Some((value, end)) => {
                result.push_str(&value);
                rest = &name[end + 1..];
            }
            None => {
                result.push('%');
                rest = name;
            }
        }
    }
    result.push_str(rest);
    result
}

/// `.` と `..` を畳み、末尾の区切りを落とす。
fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    result
}

/// 打たれた文字を、`current` を基準にしたパスへ直す。空なら `None`。
/// 「パスのコピー」で付く引用符と、スラッシュ区切りも受け付ける。
pub fn parse_path_input(text: &str, current: &Path, env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let text = text.trim().trim_matches('"').trim();
    if text.is_empty() {
        return None;
    }
    let mut text = expand_variables(text, &env).replace('/', r"\");
    // 「C:」だけだと、そのドライブの作業フォルダを指してしまう
    if matches!(text.as_bytes(), [letter, b':'] if letter.is_ascii_alphabetic()) {
        text.push('\\');
    }
    // サーバー名だけのパスは `Path` が共有名付きのパスと認めず、ドライブの下のフォルダに化ける
    if text.strip_prefix(r"\\").is_some_and(|rest| !rest.trim_end_matches('\\').contains('\\')) {
        return Some(text.trim_end_matches('\\').into());
    }
    Some(normalize(&current.join(text)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Option<String> {
        let env = |name: &str| name.eq_ignore_ascii_case("home").then(|| r"C:\Users\素材".to_string());
        parse_path_input(text, Path::new(r"C:\a\b"), env).map(|path| path.display().to_string())
    }

    #[test]
    fn absolute_paths_stay() {
        assert_eq!(parse(r"D:\写真\2026").as_deref(), Some(r"D:\写真\2026"));
    }

    #[test]
    fn blank_text_is_nothing() {
        assert_eq!(parse("   "), None);
        assert_eq!(parse(r#""""#), None);
    }

    #[test]
    fn quotes_and_spaces_around_are_dropped() {
        assert_eq!(parse("  \"C:\\Program Files\"\r ").as_deref(), Some(r"C:\Program Files"));
    }

    #[test]
    fn slashes_become_separators() {
        assert_eq!(parse("C:/Users/素材").as_deref(), Some(r"C:\Users\素材"));
    }

    #[test]
    fn a_trailing_separator_is_dropped() {
        assert_eq!(parse(r"C:\Users\").as_deref(), Some(r"C:\Users"));
    }

    #[test]
    fn drive_roots_keep_their_separator() {
        assert_eq!(parse("z:").as_deref(), Some(r"z:\"));
        assert_eq!(parse(r"Z:\").as_deref(), Some(r"Z:\"));
    }

    #[test]
    fn unc_paths_stay() {
        assert_eq!(parse(r"\\nas\共有\写真\").as_deref(), Some(r"\\nas\共有\写真"));
        assert_eq!(parse("//nas/共有/写真").as_deref(), Some(r"\\nas\共有\写真"));
    }

    #[test]
    fn unc_share_root_is_kept() {
        assert_eq!(parse(r"\\nas\共有").as_deref().map(|path| path.trim_end_matches('\\')), Some(r"\\nas\共有"));
        assert_eq!(parse(r"\\nas\共有\写真\..").as_deref().map(|path| path.trim_end_matches('\\')), Some(r"\\nas\共有"));
    }

    #[test]
    fn a_server_name_alone_stays_a_network_path() {
        assert_eq!(parse(r"\\nas").as_deref(), Some(r"\\nas"));
        assert_eq!(parse(r"\\nas\").as_deref(), Some(r"\\nas"));
    }

    #[test]
    fn relative_paths_start_at_the_current_folder() {
        assert_eq!(parse("c").as_deref(), Some(r"C:\a\b\c"));
        assert_eq!(parse(r"..\c").as_deref(), Some(r"C:\a\c"));
        assert_eq!(parse(r".\c").as_deref(), Some(r"C:\a\b\c"));
    }

    #[test]
    fn going_above_the_root_stops_at_the_root() {
        assert_eq!(parse(r"..\..\..\..").as_deref(), Some(r"C:\"));
    }

    #[test]
    fn variables_are_expanded() {
        assert_eq!(parse(r"%HOME%\写真").as_deref(), Some(r"C:\Users\素材\写真"));
    }

    #[test]
    fn unknown_variables_stay_as_typed() {
        assert_eq!(parse(r"C:\100%\%NONE%").as_deref(), Some(r"C:\100%\%NONE%"));
        assert_eq!(parse(r"C:\50%off").as_deref(), Some(r"C:\50%off"));
    }
}
