use filer_core::{Entry, Group};
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// リストの1行が、どのフォルダの何番目の項目かを表す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: u32,
    pub folder: u32,
    pub entry: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Item(TreeRow),
    Header { label: String, count: usize },
}

/// 開いているフォルダを、表示する行の並びへ展開する。
pub struct Flattened {
    pub folders: Vec<PathBuf>,
    pub rows: Vec<Row>,
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
        flat.rows.push(Row::Item(TreeRow { depth, folder: folder_ix, entry: i as u32 }));
        // 何も開いていない時は、行ごとのパスの組み立てを省く
        if entry.is_dir && lists.len() > 1 {
            walk(&folder.join(entry.os_name()), depth + 1, lists, flat);
        }
    }
}

/// `ordinal` 番目のグループの見出しより後、次の見出しより前の行の範囲。
pub fn group_range(tree: &Flattened, ordinal: usize) -> Range<usize> {
    let is_header = |row: &Row| matches!(row, Row::Header { .. });
    let mut headers = tree.rows.iter().enumerate().filter(|(_, row)| is_header(row)).map(|(ix, _)| ix);
    let Some(header) = headers.nth(ordinal) else { return 0..0 };
    (header + 1)..headers.next().unwrap_or(tree.rows.len())
}

/// 先頭の階層の項目をグループに分け、見出しの行を挟んで展開する。
pub fn flatten_grouped(
    root: &Path,
    lists: &HashMap<PathBuf, &[Entry]>,
    group_of: &dyn Fn(&Entry) -> Group,
) -> Flattened {
    let mut flat = Flattened { folders: vec![root.to_path_buf()], rows: Vec::new() };
    let Some(list) = lists.get(root) else { return flat };
    let mut groups: BTreeMap<Group, Vec<usize>> = BTreeMap::new();
    for (i, entry) in list.iter().enumerate() {
        groups.entry(group_of(entry)).or_default().push(i);
    }
    for (group, members) in groups {
        flat.rows.push(Row::Header { label: group.label, count: members.len() });
        for i in members {
            flat.rows.push(Row::Item(TreeRow { depth: 0, folder: 0, entry: i as u32 }));
            if list[i].is_dir && lists.len() > 1 {
                walk(&root.join(list[i].os_name()), 1, lists, &mut flat);
            }
        }
    }
    flat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_range_stops_at_next_header() {
        let header = || Row::Header { label: "g".into(), count: 1 };
        let item = || Row::Item(TreeRow { depth: 0, folder: 0, entry: 0 });
        let tree = Flattened { folders: vec![], rows: vec![header(), item(), item(), header(), item()] };
        assert_eq!(group_range(&tree, 0), 1..3);
        assert_eq!(group_range(&tree, 1), 4..5);
        assert_eq!(group_range(&tree, 9), 0..0);
    }

    #[test]
    fn groups_get_headers_with_counts_in_rank_order() {
        let root = [dir("late"), file("a"), file("late2")];
        let lists = HashMap::from([(PathBuf::from("r"), &root[..])]);
        let group_of = |e: &Entry| {
            let late = e.name.starts_with("late");
            Group { rank: late as u32, label: if late { "後".into() } else { "先".into() } }
        };
        let flat = flatten_grouped(Path::new("r"), &lists, &group_of);
        let headers: Vec<_> = flat
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Header { label, count } => Some((label.as_str(), *count)),
                Row::Item(_) => None,
            })
            .collect();
        assert_eq!(headers, [("先", 1), ("後", 2)]);
        assert_eq!(flat.rows.len(), 5);
        assert!(matches!(flat.rows[0], Row::Header { .. }));
    }

    fn dir(name: &str) -> Entry {
        Entry { name: name.into(), raw_name: None, is_dir: true, is_link: false, size: 0, modified: 0, created: 0 }
    }
    fn file(name: &str) -> Entry {
        Entry { name: name.into(), raw_name: None, is_dir: false, is_link: false, size: 0, modified: 0, created: 0 }
    }
    fn shape(flat: &Flattened, lists: &HashMap<PathBuf, &[Entry]>) -> Vec<(u32, String)> {
        flat.rows
            .iter()
            .filter_map(|r| match r {
                Row::Item(r) => {
                    let list = lists[&flat.folders[r.folder as usize]];
                    Some((r.depth, list[r.entry as usize].name.clone()))
                }
                Row::Header { .. } => None,
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
