use crate::Entry;

/// リストのグループ分けの基準。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupKey {
    None,
    Name,
    Modified,
    Size,
    Created,
    Author,
    Tags,
    Title,
}

impl GroupKey {
    pub const ALL: [GroupKey; 8] = [
        GroupKey::None,
        GroupKey::Name,
        GroupKey::Modified,
        GroupKey::Size,
        GroupKey::Created,
        GroupKey::Author,
        GroupKey::Tags,
        GroupKey::Title,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GroupKey::None => "なし",
            GroupKey::Name => "名前",
            GroupKey::Modified => "更新日時",
            GroupKey::Size => "サイズ",
            GroupKey::Created => "作成日時",
            GroupKey::Author => "作成者",
            GroupKey::Tags => "タグ",
            GroupKey::Title => "タイトル",
        }
    }

    /// ファイルのプロパティを読む必要があるか。
    pub fn needs_properties(self) -> bool {
        matches!(self, GroupKey::Author | GroupKey::Tags | GroupKey::Title)
    }
}

/// 現地時刻の日付。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// ファイルのプロパティのうち、グループ分けに使うもの。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    pub author: String,
    pub tags: String,
    pub title: String,
}

/// 1つのグループ。`rank`、`label` の順で並べる。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Group {
    pub rank: u32,
    pub label: String,
}

/// エクスプローラーと同じ区切りで日付を分ける。週は日曜始まり。
pub fn date_group(date: Date, today: Date) -> Group {
    let (day, now) = (days_from_civil(date), days_from_civil(today));
    let week_start = now - (now + 4).rem_euclid(7); // 1970-01-01は木曜
    let months = |d: Date| i64::from(d.year) * 12 + i64::from(d.month);
    let (rank, label) = if day > now {
        (0, "将来")
    } else if day == now {
        (1, "今日")
    } else if day == now - 1 {
        (2, "昨日")
    } else if day >= week_start {
        (3, "今週に入って（昨日は含めず）")
    } else if day >= week_start - 7 {
        (4, "先週")
    } else if months(date) == months(today) {
        (5, "今月に入って（先週は含めず）")
    } else if months(date) == months(today) - 1 {
        (6, "先月")
    } else if date.year == today.year {
        (7, "今年に入って（先月は含めず）")
    } else if date.year == today.year - 1 {
        (8, "昨年")
    } else {
        (9, "かなり前")
    };
    Group { rank, label: label.into() }
}

/// 1970-01-01からの日数。
fn days_from_civil(date: Date) -> i64 {
    let month = i64::from(date.month);
    let year = i64::from(date.year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(date.day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn size_group(entry: &Entry) -> Group {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    let (rank, label) = match entry.size {
        _ if entry.is_dir => (7, "フォルダ"),
        0 => (0, "空"),
        s if s <= 16 * KB => (1, "極小（16 KB以下）"),
        s if s <= MB => (2, "小（1 MB以下）"),
        s if s <= 128 * MB => (3, "中（128 MB以下）"),
        s if s <= GB => (4, "大（1 GB以下）"),
        s if s <= 4 * GB => (5, "特大（4 GB以下）"),
        _ => (6, "巨大（4 GB超）"),
    };
    Group { rank, label: label.into() }
}

pub fn name_group(name: &str) -> Group {
    let first = name.chars().next().unwrap_or(' ').to_ascii_uppercase();
    let (rank, label) = match first {
        '0'..='9' => (0, "0-9"),
        'A'..='H' => (1, "A-H"),
        'I'..='P' => (2, "I-P"),
        'Q'..='Z' => (3, "Q-Z"),
        '\u{3040}'..='\u{30ff}' => (4, "かな"),
        '\u{4e00}'..='\u{9fff}' => (5, "漢字"),
        _ => (6, "記号"),
    };
    Group { rank, label: label.into() }
}

/// 作成者、タグ、タイトルの値で分ける。空は最後の「指定なし」にまとめる。
pub fn value_group(value: &str) -> Group {
    match value.trim() {
        "" => Group { rank: 1, label: "指定なし".into() },
        text => Group { rank: 0, label: text.into() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }
    // 2026-11-06は金曜日。その週の日曜は11-01
    const TODAY: Date = Date { year: 2026, month: 11, day: 6 };

    fn label(date: Date) -> String {
        date_group(date, TODAY).label
    }

    #[test]
    fn date_groups_follow_explorer() {
        assert_eq!(label(d(2026, 11, 6)), "今日");
        assert_eq!(label(d(2026, 11, 5)), "昨日");
        assert_eq!(label(d(2026, 11, 1)), "今週に入って（昨日は含めず）");
        assert_eq!(label(d(2026, 10, 31)), "先週");
        assert_eq!(label(d(2026, 10, 25)), "先週");
        assert_eq!(label(d(2026, 10, 24)), "先月");
        assert_eq!(label(d(2026, 9, 30)), "今年に入って（先月は含めず）");
        assert_eq!(label(d(2026, 1, 1)), "今年に入って（先月は含めず）");
        assert_eq!(label(d(2025, 12, 31)), "昨年");
        assert_eq!(label(d(2024, 12, 31)), "かなり前");
        assert_eq!(label(d(2026, 11, 7)), "将来");
    }

    #[test]
    fn this_month_group_appears_late_in_month() {
        let today = d(2026, 11, 27); // 金曜。週の始まりは22日、先週は15日から
        assert_eq!(date_group(d(2026, 11, 14), today).label, "今月に入って（先週は含めず）");
    }

    #[test]
    fn january_treats_december_as_last_month() {
        let today = d(2027, 1, 20);
        assert_eq!(date_group(d(2026, 12, 15), today).label, "先月");
        assert_eq!(date_group(d(2026, 11, 15), today).label, "昨年");
    }

    #[test]
    fn newer_dates_rank_first() {
        assert!(date_group(d(2026, 11, 6), TODAY) < date_group(d(2026, 11, 5), TODAY));
        assert!(date_group(d(2026, 10, 31), TODAY) < date_group(d(2024, 1, 1), TODAY));
    }

    fn entry(size: u64, is_dir: bool) -> Entry {
        Entry { name: "x".into(), raw_name: None, is_dir, size, modified: 0, created: 0 }
    }

    #[test]
    fn size_groups_follow_explorer() {
        assert_eq!(size_group(&entry(0, false)).label, "空");
        assert_eq!(size_group(&entry(16_000, false)).label, "極小（16 KB以下）");
        assert_eq!(size_group(&entry(500_000, false)).label, "小（1 MB以下）");
        assert_eq!(size_group(&entry(100_000_000, false)).label, "中（128 MB以下）");
        assert_eq!(size_group(&entry(900_000_000, false)).label, "大（1 GB以下）");
        assert_eq!(size_group(&entry(3_000_000_000, false)).label, "特大（4 GB以下）");
        assert_eq!(size_group(&entry(9_000_000_000, false)).label, "巨大（4 GB超）");
        assert_eq!(size_group(&entry(0, true)).label, "フォルダ");
    }

    #[test]
    fn name_groups_by_first_character() {
        assert_eq!(name_group("2026_合作").label, "0-9");
        assert_eq!(name_group("Alpha").label, "A-H");
        assert_eq!(name_group("kiwi").label, "I-P");
        assert_eq!(name_group("Zebra").label, "Q-Z");
        assert_eq!(name_group("あい").label, "かな");
        assert_eq!(name_group("カタカナ").label, "かな");
        assert_eq!(name_group("素材").label, "漢字");
        assert_eq!(name_group(".git").label, "記号");
    }

    #[test]
    fn empty_value_goes_last() {
        assert_eq!(value_group("").label, "指定なし");
        assert!(value_group("山田") < value_group(""));
    }
}
