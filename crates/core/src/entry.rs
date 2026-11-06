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

    /// 拡張子。無ければ空文字列。先頭のドットだけの名前は拡張子と見なさない。
    pub(crate) fn extension(&self) -> &str {
        match self.name.rfind('.') {
            Some(i) if i > 0 => &self.name[i + 1..],
            _ => "",
        }
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
    use super::format_size;

    #[test]
    fn formats_size_in_units_of_1000() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(999), "999 B");
        assert_eq!(format_size(1_500), "1.5 KB");
        assert_eq!(format_size(212_000_000), "212.0 MB");
        assert_eq!(format_size(1_200_000_000), "1.2 GB");
    }
}
