//! クイックルックのツリー表示。Zipの目次とフォルダの中身を、同じ形の行にする。

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};

/// 持つ行数の上限。超えた分は読まず、途中までであることを示す。
pub const MAX_TREE_LINES: usize = 20_000;
/// 開いた状態で見せる行数の上限。超えたら最初はフォルダを閉じておく。
const OPEN_LINES: usize = 300;

/// 読み取った1項目。`path` は `/` 区切りの相対パス。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeItem {
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

/// ツリーの1行。フォルダのサイズは配下の合計。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeLine {
    pub name: String,
    pub depth: u32,
    pub is_dir: bool,
    pub size: u64,
    /// 親の行の番号
    pub parent: Option<usize>,
    pub has_children: bool,
}

/// 行の並びと合計。行は親のすぐ後に子が続く順である。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub lines: Vec<TreeLine>,
    pub files: u64,
    pub folders: u64,
    pub total: u64,
    /// 上限や時間切れで、途中までしか読んでいない
    pub cut: bool,
}

impl Tree {
    /// 最初に閉じておくフォルダ。少なければ全部開き、多ければ最上位だけを見せる。
    pub fn initially_closed(&self) -> HashSet<usize> {
        if self.lines.len() <= OPEN_LINES {
            return HashSet::new();
        }
        self.lines.iter().enumerate().filter(|(_, line)| line.has_children).map(|(index, _)| index).collect()
    }
}

struct Node {
    name: String,
    is_dir: bool,
    size: u64,
    parent: usize,
    children: HashMap<String, usize>,
}

/// 項目の一覧をツリーにする。親が無い項目には親を補う。Zipはフォルダの項目を持たないことがある。
pub fn build_tree(items: Vec<TreeItem>, cut: bool, compare: impl Fn(&str, &str) -> Ordering) -> Tree {
    let mut nodes = vec![Node { name: String::new(), is_dir: true, size: 0, parent: 0, children: HashMap::new() }];
    for item in items {
        let parts: Vec<&str> = item.path.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
        let last = parts.len().saturating_sub(1);
        let mut at = 0;
        for (index, part) in parts.iter().enumerate() {
            at = match nodes[at].children.get(*part) {
                Some(&child) => child,
                None => {
                    let child = nodes.len();
                    nodes.push(Node { name: part.to_string(), is_dir: index < last, size: 0, parent: at, children: HashMap::new() });
                    nodes[at].children.insert(part.to_string(), child);
                    child
                }
            };
            if index < last {
                nodes[at].is_dir = true;
            }
        }
        if at != 0 {
            nodes[at].is_dir |= item.is_dir;
            nodes[at].size = nodes[at].size.max(item.size);
        }
    }
    let mut tree = Tree { cut, ..Tree::default() };
    for node in &nodes[1..] {
        if node.is_dir {
            tree.folders += 1;
        } else {
            tree.files += 1;
            tree.total += node.size;
        }
    }
    // 子は親より後に作られるので、後ろから足せば合計が上へ集まる
    for index in (1..nodes.len()).rev() {
        let (size, parent) = (nodes[index].size, nodes[index].parent);
        if nodes[parent].is_dir && parent != 0 {
            nodes[parent].size += size;
        }
    }
    // 深い入れ子のファイルでスタックを使い切らないよう、再帰しない
    let mut stack: Vec<(usize, u32, Option<usize>)> = sorted_children(&nodes, 0, &compare).into_iter().rev().map(|child| (child, 0, None)).collect();
    while let Some((index, depth, parent)) = stack.pop() {
        let line = tree.lines.len();
        let node = &nodes[index];
        tree.lines.push(TreeLine { name: node.name.clone(), depth, is_dir: node.is_dir, size: node.size, parent, has_children: !node.children.is_empty() });
        for child in sorted_children(&nodes, index, &compare).into_iter().rev() {
            stack.push((child, depth + 1, Some(line)));
        }
    }
    tree
}

/// 子をフォルダ、ファイルの順に、それぞれ名前の順に並べる。
fn sorted_children(nodes: &[Node], index: usize, compare: &impl Fn(&str, &str) -> Ordering) -> Vec<usize> {
    let mut children: Vec<usize> = nodes[index].children.values().copied().collect();
    children.sort_by(|&a, &b| nodes[b].is_dir.cmp(&nodes[a].is_dir).then_with(|| compare(&nodes[a].name, &nodes[b].name)));
    children
}

/// 閉じたフォルダの配下を除いた、見える行の番号。
pub fn visible_lines(tree: &Tree, closed: &HashSet<usize>) -> Vec<usize> {
    let mut hidden = vec![false; tree.lines.len()];
    let mut visible = Vec::with_capacity(tree.lines.len());
    for (index, line) in tree.lines.iter().enumerate() {
        // 親は必ず前にあるので、親を見れば配下かどうかが分かる
        let inside = line.parent.is_some_and(|parent| hidden[parent] || closed.contains(&parent));
        hidden[index] = inside;
        if !inside {
            visible.push(index);
        }
    }
    visible
}

const END_RECORD: u32 = 0x0605_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const ZIP64_END_RECORD: u32 = 0x0606_4b50;
const DIRECTORY_ENTRY: u32 = 0x0201_4b50;
/// 終端の記録の大きさと、コメントの最大の長さ。
const END_RECORD_SIZE: u64 = 22;
const MAX_COMMENT: u64 = 65_535;
const ENTRY_HEADER: usize = 46;
const UTF8_NAME_FLAG: u16 = 1 << 11;

fn broken() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Zipの目次が読めない")
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default())
}

/// 目次の位置と項目の数。
fn find_directory(reader: &mut (impl Read + Seek)) -> io::Result<(u64, u64)> {
    let length = reader.seek(SeekFrom::End(0))?;
    if length < END_RECORD_SIZE {
        return Err(broken());
    }
    let start = length.saturating_sub(END_RECORD_SIZE + MAX_COMMENT);
    reader.seek(SeekFrom::Start(start))?;
    let mut tail = vec![0; (length - start) as usize];
    reader.read_exact(&mut tail)?;
    let at = (0..=tail.len() - END_RECORD_SIZE as usize).rev().find(|&at| u32_at(&tail, at) == END_RECORD).ok_or_else(broken)?;
    let end_position = start + at as u64;
    let (count, size, offset) = (u16_at(&tail, at + 10) as u64, u32_at(&tail, at + 12) as u64, u32_at(&tail, at + 16) as u64);
    // ZIP64は、終端の記録の直前に本当の値の場所が書いてある
    if end_position >= 20 && (count == 0xFFFF || offset == 0xFFFF_FFFF || size == 0xFFFF_FFFF) {
        reader.seek(SeekFrom::Start(end_position - 20))?;
        let mut locator = [0; 20];
        reader.read_exact(&mut locator)?;
        if u32_at(&locator, 0) == ZIP64_LOCATOR {
            reader.seek(SeekFrom::Start(u64_at(&locator, 8)))?;
            let mut record = [0; 56];
            reader.read_exact(&mut record)?;
            if u32_at(&record, 0) != ZIP64_END_RECORD {
                return Err(broken());
            }
            return Ok((u64_at(&record, 48), u64_at(&record, 32)));
        }
    }
    // 自己解凍形式は先頭に実行ファイルが付き、記録の位置がその分ずれている
    let shift = end_position.saturating_sub(size).saturating_sub(offset);
    Ok((offset + shift, count))
}

/// 拡張領域から、ZIP64の展開後のサイズを読む。
fn zip64_size(extra: &[u8]) -> Option<u64> {
    let mut at = 0;
    while at + 4 <= extra.len() {
        let (id, length) = (u16_at(extra, at), u16_at(extra, at + 2) as usize);
        at += 4;
        if id == 1 && length >= 8 && at + 8 <= extra.len() {
            return Some(u64_at(extra, at));
        }
        at += length;
    }
    None
}

fn clean_name(name: &str) -> String {
    name.replace('\\', "/")
}

/// Zipの目次を読む。中身は読まないので、大きなファイルでも読む量は目次の分だけである。
/// `shift_jis` は、UTF-8の印が無く、UTF-8としても読めない名前に使う。上限で切った時は真も返す。
pub fn read_zip_items(reader: &mut (impl Read + Seek), shift_jis: impl Fn(&[u8]) -> Option<String>) -> io::Result<(Vec<TreeItem>, bool)> {
    let (position, count) = find_directory(reader)?;
    reader.seek(SeekFrom::Start(position))?;
    let mut reader = io::BufReader::with_capacity(64 * 1024, reader);
    let mut items = Vec::new();
    for _ in 0..count {
        if items.len() >= MAX_TREE_LINES {
            return Ok((items, true));
        }
        let mut header = [0; ENTRY_HEADER];
        reader.read_exact(&mut header)?;
        if u32_at(&header, 0) != DIRECTORY_ENTRY {
            return Err(broken());
        }
        let flags = u16_at(&header, 8);
        let plain_size = u32_at(&header, 24) as u64;
        let (name_length, extra_length, comment_length) = (u16_at(&header, 28) as usize, u16_at(&header, 30) as usize, u16_at(&header, 32) as usize);
        let mut rest = vec![0; name_length + extra_length + comment_length];
        reader.read_exact(&mut rest)?;
        let raw = &rest[..name_length];
        let extra = &rest[name_length..name_length + extra_length];
        let name = match std::str::from_utf8(raw) {
            Ok(name) => name.to_string(),
            Err(_) if flags & UTF8_NAME_FLAG == 0 => shift_jis(raw).unwrap_or_else(|| String::from_utf8_lossy(raw).into_owned()),
            Err(_) => String::from_utf8_lossy(raw).into_owned(),
        };
        let path = clean_name(&name);
        let size = if plain_size == 0xFFFF_FFFF { zip64_size(extra).unwrap_or(plain_size) } else { plain_size };
        items.push(TreeItem { is_dir: path.ends_with('/'), path, size });
    }
    Ok((items, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn item(path: &str, size: u64) -> TreeItem {
        TreeItem { path: path.into(), is_dir: path.ends_with('/'), size }
    }

    fn tree(items: Vec<TreeItem>) -> Tree {
        build_tree(items, false, |a, b| a.cmp(b))
    }

    fn names(tree: &Tree) -> Vec<(u32, &str)> {
        tree.lines.iter().map(|line| (line.depth, line.name.as_str())).collect()
    }

    #[test]
    fn folders_come_first_and_children_follow_their_parent() {
        let built = tree(vec![item("z.txt", 1), item("a/b/c.txt", 2), item("a/d.txt", 3)]);
        assert_eq!(names(&built), [(0, "a"), (1, "b"), (2, "c.txt"), (1, "d.txt"), (0, "z.txt")]);
    }

    #[test]
    fn missing_parents_are_filled_and_sizes_add_up() {
        let built = tree(vec![item("a/b/c.txt", 2), item("a/d.txt", 3), item("e.txt", 5)]);
        assert_eq!(built.lines[0].size, 5);
        assert!(built.lines[0].is_dir);
        assert_eq!((built.files, built.folders, built.total), (3, 2, 10));
    }

    #[test]
    fn an_explicit_folder_item_does_not_double() {
        let built = tree(vec![item("a/", 0), item("a/b.txt", 1)]);
        assert_eq!(names(&built), [(0, "a"), (1, "b.txt")]);
        assert_eq!((built.files, built.folders), (1, 1));
    }

    #[test]
    fn empty_and_dot_parts_are_ignored() {
        let built = tree(vec![item("./a//b.txt", 1), item("/", 0)]);
        assert_eq!(names(&built), [(0, "a"), (1, "b.txt")]);
    }

    #[test]
    fn very_deep_paths_do_not_overflow_the_stack() {
        let built = tree(vec![item(&"d/".repeat(100_000), 0)]);
        assert_eq!(built.lines.len(), 100_000);
    }

    #[test]
    fn closing_a_folder_hides_everything_below_it() {
        let built = tree(vec![item("a/b/c.txt", 1), item("a/d.txt", 1), item("e.txt", 1)]);
        assert_eq!(visible_lines(&built, &HashSet::new()), [0, 1, 2, 3, 4]);
        assert_eq!(visible_lines(&built, &HashSet::from([0])), [0, 4]);
        assert_eq!(visible_lines(&built, &HashSet::from([1])), [0, 1, 3, 4]);
    }

    #[test]
    fn large_trees_start_closed() {
        let small = tree(vec![item("a/b.txt", 1)]);
        assert!(small.initially_closed().is_empty());
        let items = (0..OPEN_LINES + 1).map(|i| item(&format!("a/{i}.txt"), 1)).collect();
        let large = tree(items);
        assert_eq!(large.initially_closed(), HashSet::from([0]));
    }

    /// 目次だけのZipを作る。
    fn zip(entries: &[(&[u8], u16, u32)], prefix: usize) -> Vec<u8> {
        let mut bytes = vec![0; prefix];
        let start = bytes.len() as u32 - prefix as u32;
        let mut directory = Vec::new();
        for (name, flags, size) in entries {
            directory.extend_from_slice(&DIRECTORY_ENTRY.to_le_bytes());
            directory.extend_from_slice(&[0; 4]);
            directory.extend_from_slice(&flags.to_le_bytes());
            directory.extend_from_slice(&[0; 14]);
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 16]);
            directory.extend_from_slice(name);
        }
        bytes.extend_from_slice(&directory);
        bytes.extend_from_slice(&END_RECORD.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&start.to_le_bytes());
        bytes.extend_from_slice(&[0; 2]);
        bytes
    }

    #[test]
    fn zip_directory_is_read_with_its_sizes() {
        let bytes = zip(&[(b"a/", 0, 0), (b"a/b.txt", 0, 42)], 0);
        let (items, cut) = read_zip_items(&mut Cursor::new(bytes), |_| None).unwrap();
        assert_eq!(items, [item("a/", 0), item("a/b.txt", 42)]);
        assert!(!cut);
    }

    #[test]
    fn names_fall_back_to_shift_jis_only_without_the_utf8_mark() {
        let bytes = zip(&[(b"\x93\xFA.txt", 0, 1)], 0);
        let (items, _) = read_zip_items(&mut Cursor::new(bytes), |_| Some("日.txt".into())).unwrap();
        assert_eq!(items[0].path, "日.txt");
        let utf8 = zip(&["日.txt".as_bytes() as &[u8]].map(|name| (name, UTF8_NAME_FLAG, 1)), 0);
        let (items, _) = read_zip_items(&mut Cursor::new(utf8), |_| panic!("not needed")).unwrap();
        assert_eq!(items[0].path, "日.txt");
    }

    #[test]
    fn backslashes_are_folder_separators() {
        let bytes = zip(&[(b"a\\b.txt", 0, 1)], 0);
        let (items, _) = read_zip_items(&mut Cursor::new(bytes), |_| None).unwrap();
        assert_eq!(items[0].path, "a/b.txt");
    }

    #[test]
    fn self_extracting_prefix_is_skipped() {
        let bytes = zip(&[(b"a.txt", 0, 1)], 1000);
        let (items, _) = read_zip_items(&mut Cursor::new(bytes), |_| None).unwrap();
        assert_eq!(items[0].path, "a.txt");
    }

    #[test]
    fn broken_input_is_an_error_and_never_panics() {
        assert!(read_zip_items(&mut Cursor::new(Vec::new()), |_| None).is_err());
        assert!(read_zip_items(&mut Cursor::new(vec![0; 100]), |_| None).is_err());
        let whole = zip(&[(b"a.txt", 0, 1), (b"b.txt", 0, 2)], 0);
        for length in 0..whole.len() {
            let _ = read_zip_items(&mut Cursor::new(whole[..length].to_vec()), |_| None);
        }
        for at in 0..whole.len() {
            let mut bent = whole.clone();
            bent[at] ^= 0xFF;
            let _ = read_zip_items(&mut Cursor::new(bent), |_| None);
        }
    }

    #[test]
    fn zip64_size_is_found_in_the_extra_field() {
        let mut extra = vec![9, 0, 2, 0, 0, 0, 1, 0, 8, 0];
        extra.extend_from_slice(&5_000_000_000u64.to_le_bytes());
        assert_eq!(zip64_size(&extra), Some(5_000_000_000));
        assert_eq!(zip64_size(&[1, 0, 8, 0]), None);
    }
}
