use std::ffi::{OsStr, OsString};

/// フォルダの中の1項目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// 表示用の名前
    pub name: String,
    /// `name` へ変換すると壊れる名前の時だけ持つ、元の名前
    pub raw_name: Option<OsString>,
    pub is_dir: bool,
    /// ジャンクションやシンボリックリンク
    pub is_link: bool,
    /// 隠し属性が付いている
    pub is_hidden: bool,
    pub size: u64,
    /// 更新日時（FILETIME、1601年からの100ナノ秒）
    pub modified: i64,
    /// 作成日時（FILETIME）
    pub created: i64,
}

impl Entry {
    /// パスの組み立てと項目の区別に使う、元の名前。
    pub fn os_name(&self) -> &OsStr {
        self.raw_name.as_deref().unwrap_or(OsStr::new(&self.name))
    }

    /// 別の場所を指す項目か。印を付けて示す。
    pub fn is_shortcut(&self) -> bool {
        self.is_link || ["lnk", "url"].iter().any(|ext| self.extension().eq_ignore_ascii_case(ext))
    }

    /// 拡張子。無ければ空文字列。先頭のドットだけの名前は拡張子と見なさない。
    pub(crate) fn extension(&self) -> &str {
        match self.name.rfind('.') {
            Some(i) if i > 0 => &self.name[i + 1..],
            _ => "",
        }
    }
}

/// 検索の条件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    /// 小文字にした語。すべてを含む名前が一致する
    pub terms: Vec<String>,
    /// 配下のフォルダも探す
    pub recursive: bool,
    /// 隠し属性のフォルダの中も探す
    pub hidden: bool,
}

impl SearchQuery {
    /// 検索欄の文字から作る。空白で区切った語をすべて含む名前を探す。
    pub fn new(text: &str, recursive: bool) -> Self {
        Self { terms: text.to_lowercase().split_whitespace().map(str::to_string).collect(), recursive, hidden: false }
    }

    pub fn matches(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.terms.iter().all(|term| name.contains(term.as_str()))
    }
}

/// サイズの列に出す文字列。Finderに合わせて1000単位で表す。
pub fn format_size(size: u64) -> String {
    if size < 1000 {
        return format!("{size} B");
    }
    let mut value = size as f64 / 1000.0;
    for unit in ["KB", "MB", "GB"] {
        if value < 1000.0 {
            return format!("{value:.1} {unit}");
        }
        value /= 1000.0;
    }
    format!("{value:.1} TB")
}

/// 種類の列に出す文字列。ファイルは拡張子を小文字にしたもの。
pub fn kind_of(entry: &Entry) -> String {
    if entry.is_dir {
        return "フォルダ".into();
    }
    match entry.extension() {
        "" => "ファイル".into(),
        ext => ext.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, SearchQuery, format_size};

    #[test]
    fn search_needs_every_word_and_ignores_case() {
        let query = SearchQuery::new("  Report 2026　", true);
        assert_eq!(query.terms, ["report", "2026"]);
        assert!(query.matches("年次REPORT-2026.pdf"));
        assert!(!query.matches("report-2025.pdf"));
        assert!(SearchQuery::new(" ", true).terms.is_empty());
    }

    fn named(name: &str, is_link: bool) -> Entry {
        Entry { name: name.into(), raw_name: None, is_dir: false, is_link, is_hidden: false, size: 0, modified: 0, created: 0 }
    }

    #[test]
    fn links_and_shortcut_files_count_as_shortcuts() {
        assert!(named("a.lnk", false).is_shortcut());
        assert!(named("A.LNK", false).is_shortcut());
        assert!(named("site.url", false).is_shortcut());
        assert!(named("junction", true).is_shortcut());
        assert!(!named("a.txt", false).is_shortcut());
        assert!(!named(".lnk", false).is_shortcut());
    }

    #[test]
    fn formats_size_in_units_of_1000() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1_500), "1.5 KB");
        assert_eq!(format_size(212_000_000), "212.0 MB");
        assert_eq!(format_size(1_200_000_000), "1.2 GB");
    }
}
