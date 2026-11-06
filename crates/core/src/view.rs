use crate::{GroupKey, SortKey, SortOrder};
use serde::{Deserialize, Serialize};

pub const MIN_FIELD_WIDTH: f32 = 40.0;
pub const MAX_FIELD_WIDTH: f32 = 1200.0;
pub const DEFAULT_COLUMN_WIDTH: f32 = 230.0;
pub const MIN_ICON_SIZE: f32 = 32.0;
pub const MAX_ICON_SIZE: f32 = 256.0;
pub const DEFAULT_ICON_SIZE: f32 = 72.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    Columns,
    List,
    Icons,
}

/// ドラッグ中にフォルダの上で止まった時の動き。カラム表示は常に列を開く。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DragHover {
    /// フォルダを開く
    Open,
    /// 中身を小さな窓で見せる
    Popup,
}

/// アプリ全体の設定。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// アイコン表示の選択を、マウスオーバーと同じくセル全体で示す
    pub grid_selection_matches_hover: bool,
    pub drag_hover: DragHover,
    /// ドラッグ中、最初のフォルダを開くまでの待ち時間（ミリ秒）
    pub spring_first_ms: u64,
    /// ドラッグ中、2回目以降に開くまでの待ち時間（ミリ秒）
    pub spring_next_ms: u64,
    pub long_name: LongName,
}

/// アイコン表示で名前が収まらない時の見せ方。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LongName {
    /// 最後の行を薄くして消す
    Fade,
    /// マウスを乗せた間だけ流して見せる
    Scroll,
}

/// 待ち時間の選択肢（ミリ秒）。
pub const SPRING_CHOICES: [u64; 5] = [500, 1000, 1500, 2000, 3000];

/// 次の選択肢へ進める。最後の次は最初に戻る。
pub fn next_spring_delay(current: u64) -> u64 {
    let next = SPRING_CHOICES.iter().find(|&&choice| choice > current);
    next.copied().unwrap_or(SPRING_CHOICES[0])
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            grid_selection_matches_hover: true,
            drag_hover: DragHover::Open,
            spring_first_ms: 1000,
            spring_next_ms: 1500,
            long_name: LongName::Fade,
        }
    }
}

/// リストに出せる項目。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Field {
    Name,
    Modified,
    Created,
    Size,
    Kind,
    Author,
    Tags,
    Title,
}

impl Field {
    pub const ALL: [Field; 8] = [
        Field::Name,
        Field::Modified,
        Field::Created,
        Field::Size,
        Field::Kind,
        Field::Author,
        Field::Tags,
        Field::Title,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Name => "名前",
            Field::Modified => "更新日時",
            Field::Created => "作成日時",
            Field::Size => "サイズ",
            Field::Kind => "種類",
            Field::Author => "作成者",
            Field::Tags => "タグ",
            Field::Title => "タイトル",
        }
    }

    pub fn default_width(self) -> f32 {
        match self {
            Field::Name => 320.0,
            Field::Modified | Field::Created => 150.0,
            Field::Size => 90.0,
            Field::Kind => 110.0,
            Field::Author | Field::Tags | Field::Title => 140.0,
        }
    }

    /// この項目で並べ替えられる時の基準。
    pub fn sort_key(self) -> Option<SortKey> {
        match self {
            Field::Name => Some(SortKey::Name),
            Field::Modified => Some(SortKey::Modified),
            Field::Created => Some(SortKey::Created),
            Field::Size => Some(SortKey::Size),
            Field::Kind => Some(SortKey::Kind),
            Field::Author | Field::Tags | Field::Title => None,
        }
    }

    pub fn needs_properties(self) -> bool {
        matches!(self, Field::Author | Field::Tags | Field::Title)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListColumn {
    pub field: Field,
    pub width: f32,
}

/// フォルダごとに覚える表示の設定。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderView {
    pub mode: ViewMode,
    pub order: SortOrder,
    pub group: GroupKey,
    pub columns: Vec<ListColumn>,
    pub column_width: f32,
    pub icon_size: f32,
}

impl Default for FolderView {
    fn default() -> Self {
        let columns = [Field::Name, Field::Modified, Field::Size, Field::Kind]
            .map(|field| ListColumn { field, width: field.default_width() })
            .to_vec();
        Self {
            mode: ViewMode::List,
            order: SortOrder { key: SortKey::Name, descending: false },
            group: GroupKey::None,
            columns,
            column_width: DEFAULT_COLUMN_WIDTH,
            icon_size: DEFAULT_ICON_SIZE,
        }
    }
}

impl FolderView {
    /// 開くフォルダの設定。保存が無ければ、今の表示形式を引き継いだ既定の設定にする。
    pub fn for_folder(saved: Option<FolderView>, current_mode: ViewMode) -> Self {
        match saved {
            Some(mut view) => {
                view.repair();
                view
            }
            None => Self { mode: current_mode, ..Self::default() },
        }
    }

    /// 壊れた保存値で画面が使えなくならないよう、必須の項目と幅を直す。
    fn repair(&mut self) {
        if !self.has_field(Field::Name) {
            let field = Field::Name;
            self.columns.insert(0, ListColumn { field, width: field.default_width() });
        }
        for column in &mut self.columns {
            column.width = column.width.clamp(MIN_FIELD_WIDTH, MAX_FIELD_WIDTH);
        }
        if !(MIN_FIELD_WIDTH..=MAX_FIELD_WIDTH).contains(&self.column_width) {
            self.column_width = DEFAULT_COLUMN_WIDTH;
        }
        if !(MIN_ICON_SIZE..=MAX_ICON_SIZE).contains(&self.icon_size) {
            self.icon_size = DEFAULT_ICON_SIZE;
        }
    }

    /// 項目の表示を切り替える。名前は消さない。
    pub fn toggle_field(&mut self, field: Field) {
        if field == Field::Name {
            return;
        }
        match self.columns.iter().position(|column| column.field == field) {
            Some(index) => {
                self.columns.remove(index);
            }
            None => self.columns.push(ListColumn { field, width: field.default_width() }),
        }
    }

    pub fn move_column(&mut self, from: usize, to: usize) {
        if from < self.columns.len() && to < self.columns.len() {
            let column = self.columns.remove(from);
            self.columns.insert(to, column);
        }
    }

    pub fn resize_column(&mut self, index: usize, width: f32) {
        if let Some(column) = self.columns.get_mut(index) {
            column.width = width.clamp(MIN_FIELD_WIDTH, MAX_FIELD_WIDTH);
        }
    }

    pub fn needs_properties(&self) -> bool {
        self.group.needs_properties()
            || (self.mode == ViewMode::List && self.columns.iter().any(|column| column.field.needs_properties()))
    }

    pub fn has_field(&self, field: Field) -> bool {
        self.columns.iter().any(|column| column.field == field)
    }
}

/// 高さが同じ行の並びで、範囲選択の枠が触れている行。
pub fn rows_in_band(y0: f32, y1: f32, row_height: f32, count: usize) -> std::ops::Range<usize> {
    let (top, bottom) = (y0.min(y1), y0.max(y1));
    if bottom < 0.0 {
        return 0..0;
    }
    let first = (top.max(0.0) / row_height) as usize;
    let last = (bottom / row_height) as usize + 1;
    first.min(count)..last.min(count)
}

/// 文字列の表示幅の見積もり。全角は1文字分、半角はその6割で数える。
pub fn text_width(text: &str, font_size: f32) -> f32 {
    let (half, full) = text.chars().fold((0u32, 0u32), |(half, full), c| {
        if c.is_ascii() { (half + 1, full) } else { (half, full + 1) }
    });
    (half as f32 * font_size * 3.0 / 5.0) + full as f32 * font_size
}

/// いちばん長い文字列が収まる幅。
pub fn fit_width<'a>(texts: impl Iterator<Item = &'a str>, font_size: f32, padding: f32) -> f32 {
    let longest = texts.map(|text| text_width(text, font_size)).fold(0.0, f32::max);
    (longest + padding).clamp(MIN_FIELD_WIDTH, MAX_FIELD_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(view: &FolderView) -> Vec<Field> {
        view.columns.iter().map(|c| c.field).collect()
    }

    #[test]
    fn toggling_adds_and_removes_a_field() {
        let mut view = FolderView::default();
        view.toggle_field(Field::Author);
        assert!(view.has_field(Field::Author));
        assert!(view.needs_properties());
        view.toggle_field(Field::Author);
        assert!(!view.has_field(Field::Author));
        assert!(!view.needs_properties());
    }

    #[test]
    fn name_cannot_be_removed() {
        let mut view = FolderView::default();
        view.toggle_field(Field::Name);
        assert!(view.has_field(Field::Name));
    }

    #[test]
    fn grouping_by_author_needs_properties() {
        let view = FolderView { group: GroupKey::Author, ..FolderView::default() };
        assert!(view.needs_properties());
    }

    #[test]
    fn moving_a_column_shifts_the_others() {
        let mut view = FolderView::default();
        view.move_column(3, 0);
        assert_eq!(fields(&view), [Field::Kind, Field::Name, Field::Modified, Field::Size]);
        view.move_column(0, 3);
        assert_eq!(fields(&view), [Field::Name, Field::Modified, Field::Size, Field::Kind]);
    }

    #[test]
    fn resizing_keeps_limits() {
        let mut view = FolderView::default();
        view.resize_column(0, 5.0);
        assert_eq!(view.columns[0].width, MIN_FIELD_WIDTH);
        view.resize_column(0, 9999.0);
        assert_eq!(view.columns[0].width, MAX_FIELD_WIDTH);
    }

    #[test]
    fn full_width_text_counts_wider_than_half_width() {
        assert_eq!(text_width("素材", 10.0), 20.0);
        assert_eq!(text_width("ab", 10.0), 12.0);
    }

    #[test]
    fn fit_width_uses_longest_text_with_padding() {
        assert_eq!(fit_width(["ab", "素材"].into_iter(), 10.0, 30.0), 50.0);
        assert_eq!(fit_width(std::iter::empty(), 10.0, 5.0), MIN_FIELD_WIDTH);
    }

    #[test]
    fn list_fields_need_properties_only_in_list() {
        let mut view = FolderView::default();
        view.toggle_field(Field::Author);
        view.mode = ViewMode::Icons;
        assert!(!view.needs_properties());
    }

    #[test]
    fn unsaved_folder_inherits_current_mode() {
        let view = FolderView::for_folder(None, ViewMode::Columns);
        assert_eq!(view.mode, ViewMode::Columns);
        let saved = FolderView { mode: ViewMode::Icons, ..FolderView::default() };
        assert_eq!(FolderView::for_folder(Some(saved), ViewMode::Columns).mode, ViewMode::Icons);
    }

    #[test]
    fn spring_delay_cycles_through_choices() {
        assert_eq!(next_spring_delay(500), 1000);
        assert_eq!(next_spring_delay(3000), 500);
        assert_eq!(next_spring_delay(700), 1000);
    }

    #[test]
    fn row_band_covers_rows_it_touches() {
        assert_eq!(rows_in_band(10.0, 30.0, 24.0, 10), 0..2);
        assert_eq!(rows_in_band(30.0, 10.0, 24.0, 10), 0..2);
        assert_eq!(rows_in_band(50.0, 500.0, 24.0, 4), 2..4);
        assert_eq!(rows_in_band(-40.0, -10.0, 24.0, 4), 0..0);
    }

    #[test]
    fn broken_saved_view_is_repaired() {
        let broken = FolderView { columns: Vec::new(), column_width: 1.0, ..FolderView::default() };
        let view = FolderView::for_folder(Some(broken), ViewMode::List);
        assert!(view.has_field(Field::Name));
        assert_eq!(view.column_width, DEFAULT_COLUMN_WIDTH);
    }

    #[test]
    fn old_saved_view_without_new_fields_still_loads() {
        let view: FolderView = serde_json::from_str(r#"{"mode":"Icons"}"#).unwrap();
        assert_eq!(view.mode, ViewMode::Icons);
        assert_eq!(view.group, GroupKey::None);
    }

    #[test]
    fn view_survives_serialization() {
        let mut view = FolderView::default();
        view.toggle_field(Field::Tags);
        view.group = GroupKey::Modified;
        let text = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<FolderView>(&text).unwrap(), view);
    }
}
