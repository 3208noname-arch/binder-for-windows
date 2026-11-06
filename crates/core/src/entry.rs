/// フォルダの中の1項目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// 更新日時（FILETIME、1601年からの100ナノ秒）
    pub modified: i64,
}

impl Entry {
    /// 拡張子。無ければ空文字列。先頭のドットだけの名前は拡張子と見なさない。
    pub(crate) fn extension(&self) -> &str {
        match self.name.rfind('.') {
            Some(i) if i > 0 => &self.name[i + 1..],
            _ => "",
        }
    }
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
