//! サイドバーの「よく使う項目」の並び。既定の項目も登録した項目も同じように扱う。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Favorite {
    pub name: String,
    pub path: PathBuf,
}

impl Favorite {
    /// フォルダ名を表示名にする。ドライブのように名前が無ければパスをそのまま使う。
    pub fn new(path: PathBuf) -> Self {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        Self { name, path }
    }
}

/// Windowsのパスは大文字と小文字を区別せず、末尾の区切りの有無も同じ場所を指す。
fn same_path(a: &Path, b: &Path) -> bool {
    let key = |path: &Path| path.to_string_lossy().trim_end_matches('\\').to_lowercase();
    key(a) == key(b)
}

pub fn is_favorite(list: &[Favorite], path: &Path) -> bool {
    list.iter().any(|favorite| same_path(&favorite.path, path))
}

/// 末尾へ足す。登録済みのフォルダは動かさない。
pub fn add_favorites(list: &mut Vec<Favorite>, paths: &[PathBuf]) {
    for path in paths {
        if !is_favorite(list, path) {
            list.push(Favorite::new(path.clone()));
        }
    }
}

/// `gap` 番目の行の前へ置く。登録済みのフォルダは名前を保ったまま動かす。
pub fn place_favorites(list: &mut Vec<Favorite>, paths: &[PathBuf], gap: usize) {
    let mut gap = gap.min(list.len());
    let mut placed: Vec<Favorite> = Vec::new();
    for path in paths {
        if is_favorite(&placed, path) {
            continue;
        }
        placed.push(match list.iter().position(|favorite| same_path(&favorite.path, path)) {
            Some(index) => {
                gap -= usize::from(index < gap);
                list.remove(index)
            }
            None => Favorite::new(path.clone()),
        });
    }
    list.splice(gap..gap, placed);
}

pub fn remove_favorite(list: &mut Vec<Favorite>, path: &Path) {
    list.retain(|favorite| !same_path(&favorite.path, path));
}

/// 空の名前では変えない。
pub fn rename_favorite(list: &mut [Favorite], path: &Path, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    if let Some(favorite) = list.iter_mut().find(|favorite| same_path(&favorite.path, path)) {
        favorite.name = name.to_string();
    }
}

/// ドラッグが行のどこに乗っているか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone {
    Before,
    Into,
    After,
}

/// 行の上下の端は行の間、中央はそのフォルダの中を指す。
pub fn drop_zone(offset: f32, height: f32) -> DropZone {
    const EDGE: f32 = 0.3;
    if offset < height * EDGE {
        DropZone::Before
    } else if offset > height * (1. - EDGE) {
        DropZone::After
    } else {
        DropZone::Into
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(paths: &[&str]) -> Vec<Favorite> {
        paths.iter().map(|path| Favorite::new(PathBuf::from(path))).collect()
    }

    fn paths(list: &[Favorite]) -> Vec<String> {
        list.iter().map(|favorite| favorite.path.display().to_string()).collect()
    }

    #[test]
    fn the_folder_name_becomes_the_label() {
        assert_eq!(Favorite::new(PathBuf::from(r"C:\素材\写真")).name, "写真");
        assert_eq!(Favorite::new(PathBuf::from(r"D:\")).name, r"D:\");
        assert_eq!(Favorite::new(PathBuf::from(r"\\nas\共有")).name, r"\\nas\共有");
    }

    #[test]
    fn adding_ignores_case_and_trailing_separators() {
        let mut favorites = list(&[r"C:\Users"]);
        add_favorites(&mut favorites, &[r"c:\users\".into(), r"C:\a".into(), r"C:\A".into()]);
        assert_eq!(paths(&favorites), [r"C:\Users", r"C:\a"]);
    }

    #[test]
    fn placing_a_new_folder_inserts_at_the_gap() {
        let mut favorites = list(&[r"C:\a", r"C:\b"]);
        place_favorites(&mut favorites, &[r"C:\n".into()], 1);
        assert_eq!(paths(&favorites), [r"C:\a", r"C:\n", r"C:\b"]);
    }

    #[test]
    fn placing_past_the_end_appends() {
        let mut favorites = list(&[r"C:\a"]);
        place_favorites(&mut favorites, &[r"C:\n".into()], 9);
        assert_eq!(paths(&favorites), [r"C:\a", r"C:\n"]);
    }

    #[test]
    fn moving_down_counts_the_removed_row() {
        let mut favorites = list(&[r"C:\a", r"C:\b", r"C:\c"]);
        place_favorites(&mut favorites, &[r"C:\a".into()], 2);
        assert_eq!(paths(&favorites), [r"C:\b", r"C:\a", r"C:\c"]);
    }

    #[test]
    fn moving_up_keeps_the_gap() {
        let mut favorites = list(&[r"C:\a", r"C:\b", r"C:\c"]);
        place_favorites(&mut favorites, &[r"C:\c".into()], 0);
        assert_eq!(paths(&favorites), [r"C:\c", r"C:\a", r"C:\b"]);
    }

    #[test]
    fn dropping_a_row_next_to_itself_changes_nothing() {
        for gap in [1, 2] {
            let mut favorites = list(&[r"C:\a", r"C:\b", r"C:\c"]);
            place_favorites(&mut favorites, &[r"C:\b".into()], gap);
            assert_eq!(paths(&favorites), [r"C:\a", r"C:\b", r"C:\c"]);
        }
    }

    #[test]
    fn moving_keeps_the_given_name() {
        let mut favorites = list(&[r"C:\a", r"C:\b"]);
        rename_favorite(&mut favorites, Path::new(r"C:\b"), "仕事");
        place_favorites(&mut favorites, &[r"c:\B".into()], 0);
        assert_eq!(favorites[0].name, "仕事");
    }

    #[test]
    fn several_folders_stay_in_order_and_unique() {
        let mut favorites = list(&[r"C:\a"]);
        place_favorites(&mut favorites, &[r"C:\x".into(), r"C:\y".into(), r"C:\X".into()], 0);
        assert_eq!(paths(&favorites), [r"C:\x", r"C:\y", r"C:\a"]);
    }

    #[test]
    fn removing_ignores_case() {
        let mut favorites = list(&[r"C:\a", r"C:\b"]);
        remove_favorite(&mut favorites, Path::new(r"c:\A"));
        assert_eq!(paths(&favorites), [r"C:\b"]);
        remove_favorite(&mut favorites, Path::new(r"C:\none"));
        assert_eq!(favorites.len(), 1);
    }

    #[test]
    fn an_empty_name_is_not_applied() {
        let mut favorites = list(&[r"C:\a"]);
        rename_favorite(&mut favorites, Path::new(r"C:\a"), "   ");
        assert_eq!(favorites[0].name, "a");
        rename_favorite(&mut favorites, Path::new(r"C:\a"), " 素材 ");
        assert_eq!(favorites[0].name, "素材");
    }

    #[test]
    fn row_edges_are_gaps_and_the_middle_is_the_folder() {
        assert_eq!(drop_zone(2., 22.), DropZone::Before);
        assert_eq!(drop_zone(11., 22.), DropZone::Into);
        assert_eq!(drop_zone(20., 22.), DropZone::After);
    }
}
