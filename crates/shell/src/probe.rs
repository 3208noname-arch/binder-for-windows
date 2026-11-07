//! パス欄から開く前に、パスの先を確かめる。

use std::ffi::OsString;
use std::io::ErrorKind;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use windows::Win32::Storage::FileSystem::{FindClose, FindFirstFileW, WIN32_FIND_DATAW};
use windows::core::PCWSTR;

/// パスの先にあるもの。見つかった時のパスは実際の大文字と小文字に直してある。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probed {
    Folder(PathBuf),
    File(PathBuf),
    Missing,
    Denied,
}

/// 実際のファイル名を読む。読めなければ `None`。
fn stored_name(path: &Path) -> Option<OsString> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut data = WIN32_FIND_DATAW::default();
    unsafe {
        let find = FindFirstFileW(PCWSTR(wide.as_ptr()), &mut data).ok()?;
        let _ = FindClose(find);
    }
    let length = data.cFileName.iter().position(|&c| c == 0)?;
    Some(OsString::from_wide(&data.cFileName[..length]))
}

/// 打たれたままの綴りだと、タブの題名やサイドバーの印が実際の名前とずれる。
fn stored_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                let stored = stored_name(&result.join(name));
                result.push(stored.as_deref().unwrap_or(name));
            }
            other => result.push(other),
        }
    }
    result
}

/// 切断中のNASでは長く待つので、画面の処理の中では呼ばない。
pub fn probe_path(path: &Path) -> Probed {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Probed::Folder(stored_path(path)),
        Ok(_) => Probed::File(stored_path(path)),
        Err(error) if error.kind() == ErrorKind::PermissionDenied => Probed::Denied,
        Err(_) => Probed::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_come_back_with_their_stored_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("MixedCase素材")).unwrap();
        let found = probe_path(&dir.path().join("mixedcase素材"));
        let Probed::Folder(path) = found else { panic!("{found:?}") };
        assert_eq!(path.file_name().unwrap(), "MixedCase素材");
    }

    #[test]
    fn files_are_told_from_folders() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(matches!(probe_path(&file), Probed::File(_)));
    }

    #[test]
    fn missing_paths_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(probe_path(&dir.path().join("none")), Probed::Missing);
    }

    #[test]
    fn invalid_characters_are_missing_not_a_crash() {
        assert_eq!(probe_path(Path::new(r"C:\a?b*c")), Probed::Missing);
        assert_eq!(probe_path(Path::new(r"\\")), Probed::Missing);
    }

    #[test]
    fn a_drive_root_is_a_folder() {
        assert_eq!(probe_path(Path::new(r"C:\")), Probed::Folder(r"C:\".into()));
    }
}
