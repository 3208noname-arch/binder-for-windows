use filer_core::Entry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// リストの1行が、どのフォルダの何番目の項目かを表す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: u32,
    pub folder: u32,
    pub entry: u32,
}

/// 開いているフォルダを、表示する行の並びへ展開する。
pub struct Flattened {
    pub folders: Vec<PathBuf>,
    pub rows: Vec<TreeRow>,
}

pub fn flatten(root: &Path, lists: &HashMap<PathBuf, &[Entry]>) -> Flattened {
    let mut flat = Flattened { folders: Vec::new(), rows: Vec::new() };
    walk(root, 0, lists, &mut flat);
    flat
}

fn walk(folder: &Path, depth: u32, lists: &HashMap<PathBuf, &[Entry]>, flat: &mut Flattened) {
    let Some(list) = lists.get(folder) else { return };
    let folder_ix = flat.folders.len() as u32;
    flat.folders.push(folder.to_path_buf());
    for (i, entry) in list.iter().enumerate() {
        flat.rows.push(TreeRow { depth, folder: folder_ix, entry: i as u32 });
        // 何も開いていない時は、行ごとのパスの組み立てを省く
        if entry.is_dir && lists.len() > 1 {
            walk(&folder.join(entry.os_name()), depth + 1, lists, flat);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> Entry {
        Entry { name: name.into(), raw_name: None, is_dir: true, size: 0, modified: 0 }
    }
    fn file(name: &str) -> Entry {
        Entry { name: name.into(), raw_name: None, is_dir: false, size: 0, modified: 0 }
    }
    fn shape(flat: &Flattened, lists: &HashMap<PathBuf, &[Entry]>) -> Vec<(u32, String)> {
        flat.rows
            .iter()
            .map(|r| {
                let list = lists[&flat.folders[r.folder as usize]];
                (r.depth, list[r.entry as usize].name.clone())
            })
            .collect()
    }

    #[test]
    fn closed_folder_shows_only_its_row() {
        let root = [dir("a"), file("b")];
        let lists = HashMap::from([(PathBuf::from("r"), &root[..])]);
        let flat = flatten(Path::new("r"), &lists);
        assert_eq!(shape(&flat, &lists), [(0, "a".to_string()), (0, "b".to_string())]);
    }

    #[test]
    fn open_folder_puts_children_under_it() {
        let root = [dir("a"), file("b")];
        let a = [file("x"), dir("y")];
        let y = [file("z")];
        let lists = HashMap::from([
            (PathBuf::from("r"), &root[..]),
            (Path::new("r").join("a"), &a[..]),
            (Path::new("r").join("a").join("y"), &y[..]),
        ]);
        let flat = flatten(Path::new("r"), &lists);
        let expected = [(0, "a"), (1, "x"), (1, "y"), (2, "z"), (0, "b")];
        let expected: Vec<_> = expected.iter().map(|(d, n)| (*d, n.to_string())).collect();
        assert_eq!(shape(&flat, &lists), expected);
    }

    #[test]
    fn file_with_same_path_as_key_is_not_expanded() {
        let root = [file("a")];
        let a = [file("x")];
        let lists = HashMap::from([(PathBuf::from("r"), &root[..]), (Path::new("r").join("a"), &a[..])]);
        let flat = flatten(Path::new("r"), &lists);
        assert_eq!(flat.rows.len(), 1);
    }
}
