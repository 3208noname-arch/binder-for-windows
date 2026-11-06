use crate::Entry;
use rayon::slice::ParallelSliceMut;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Modified,
    Size,
    Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortOrder {
    pub key: SortKey,
    pub descending: bool,
}

pub type NameCmp = fn(&str, &str) -> Ordering;

/// フォルダを常に前に置き、同じ値の時は名前の昇順で決める。
fn compare(a: &Entry, b: &Entry, order: SortOrder, cmp: NameCmp) -> Ordering {
    b.is_dir.cmp(&a.is_dir).then_with(|| {
        let by_key = match order.key {
            SortKey::Name => cmp(&a.name, &b.name),
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Kind => compare_ignore_case(a.extension(), b.extension()),
        };
        let by_key = if order.descending { by_key.reverse() } else { by_key };
        by_key.then_with(|| cmp(&a.name, &b.name))
    })
}

fn compare_ignore_case(a: &str, b: &str) -> Ordering {
    a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase))
}

pub fn sort_entries(entries: &mut [Entry], order: SortOrder, cmp: NameCmp) {
    entries.par_sort_by(|a, b| compare(a, b, order, cmp));
}

/// 並べ替え済みの `entries` へ `batch` を併合する。
pub fn insert_sorted(entries: &mut Vec<Entry>, mut batch: Vec<Entry>, order: SortOrder, cmp: NameCmp) {
    sort_entries(&mut batch, order, cmp);
    let mut merged = Vec::with_capacity(entries.len() + batch.len());
    let mut left = std::mem::take(entries).into_iter().peekable();
    let mut right = batch.into_iter().peekable();
    while let (Some(l), Some(r)) = (left.peek(), right.peek()) {
        let take_left = compare(l, r, order, cmp) != Ordering::Greater;
        merged.extend(if take_left { left.next() } else { right.next() });
    }
    merged.extend(left);
    merged.extend(right);
    *entries = merged;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind_of;

    fn file(name: &str, size: u64, modified: i64) -> Entry {
        Entry { name: name.into(), is_dir: false, size, modified }
    }
    fn dir(name: &str) -> Entry {
        Entry { name: name.into(), is_dir: true, size: 0, modified: 0 }
    }
    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }
    const BY_NAME: SortOrder = SortOrder { key: SortKey::Name, descending: false };
    fn cmp(a: &str, b: &str) -> Ordering {
        a.cmp(b)
    }

    #[test]
    fn sorts_folders_first_by_name() {
        let mut e = vec![file("a", 0, 0), dir("z"), file("b", 0, 0), dir("m")];
        sort_entries(&mut e, BY_NAME, cmp);
        assert_eq!(names(&e), ["m", "z", "a", "b"]);
    }

    #[test]
    fn sorts_by_modified_descending() {
        let mut e = vec![file("a", 0, 1), file("b", 0, 3), file("c", 0, 2)];
        sort_entries(&mut e, SortOrder { key: SortKey::Modified, descending: true }, cmp);
        assert_eq!(names(&e), ["b", "c", "a"]);
    }

    #[test]
    fn descending_keeps_folders_first() {
        let mut e = vec![file("a", 0, 0), dir("m"), dir("z")];
        sort_entries(&mut e, SortOrder { key: SortKey::Name, descending: true }, cmp);
        assert_eq!(names(&e), ["z", "m", "a"]);
    }

    #[test]
    fn same_key_falls_back_to_name() {
        let mut e = vec![file("b", 5, 0), file("a", 5, 0)];
        sort_entries(&mut e, SortOrder { key: SortKey::Size, descending: false }, cmp);
        assert_eq!(names(&e), ["a", "b"]);
    }

    #[test]
    fn insert_sorted_keeps_order() {
        let mut e = vec![file("a", 0, 0), file("c", 0, 0)];
        insert_sorted(&mut e, vec![file("d", 0, 0), file("b", 0, 0)], BY_NAME, cmp);
        assert_eq!(names(&e), ["a", "b", "c", "d"]);
    }

    #[test]
    fn kind_is_folder_or_lowercase_extension() {
        assert_eq!(kind_of(&dir("x.y")), "フォルダ");
        assert_eq!(kind_of(&file("a.MP4", 0, 0)), "mp4");
        assert_eq!(kind_of(&file("Makefile", 0, 0)), "ファイル");
    }
}
