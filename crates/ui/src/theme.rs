//! 色と寸法の定義。Windows風の見た目を足す時は、ここを切り替える。

pub const FONT: &str = "Yu Gothic UI";
pub const TEXT_SIZE: f32 = 13.0;
pub const SMALL_TEXT_SIZE: f32 = 12.0;
pub const ROW_HEIGHT: f32 = 24.0;
pub const TOOLBAR_HEIGHT: f32 = 44.0;
pub const STATUS_HEIGHT: f32 = 26.0;
pub const TAB_BAR_HEIGHT: f32 = 28.0;
/// リストの右端の、押すと選択を解除する帯
pub const CLEAR_ZONE_WIDTH: f32 = 28.0;
/// 移動の直前に点滅する色
pub const FLASH: u32 = 0xFFD76A;
/// ドラッグ中に端で止まった時に送る速さ（毎秒のピクセル）
pub const DRAG_EDGE_SPEED: f32 = 480.0;
/// ドラッグ中に端と見なす幅
pub const DRAG_EDGE_WIDTH: f32 = 36.0;
/// ドラッグ中のホイール1目盛りで送る量
pub const DRAG_WHEEL_STEP: f32 = 72.0;
pub const TAB_CLOSE_WIDTH: f32 = 18.0;
pub const TAB_NEW_WIDTH: f32 = 32.0;
pub const SIDEBAR_WIDTH: f32 = 190.0;
pub const COLUMN_MIN_WIDTH: f32 = 120.0;
pub const COLUMN_MAX_WIDTH: f32 = 640.0;
pub const COLUMN_HANDLE_WIDTH: f32 = 6.0;
pub const COLUMN_FIT_PADDING: f32 = 46.0;
pub const ROW_SIDE_PADDING: f32 = 12.0;
pub const NAME_FIT_PADDING: f32 = 48.0;
pub const FIELD_FIT_PADDING: f32 = 18.0;
pub const FIELD_GAP: f32 = 8.0;
pub const DRAG_THRESHOLD: f32 = 5.0;
pub const GRAB_MIN_WIDTH: f32 = 160.0;
pub const COLUMN_ROW_SPARE: f32 = 40.0;
pub const MENU_RIGHT_SORT: f32 = 488.0;
pub const CUT_OPACITY: f32 = 0.5;
pub const SCROLL_WIDTH: f32 = 6.0;
pub const SCROLL_MARGIN: f32 = 2.0;
pub const SCROLL_MIN_THUMB: f32 = 28.0;
pub const SCROLL_THUMB: u32 = 0x00000055;
pub const SCROLL_THUMB_HOVER: u32 = 0x00000088;
pub const BAND_FILL: u32 = 0x0a64d833;
pub const BAND_LINE: u32 = 0x0a64d8;
pub const ICON_SIZE_STEP: f32 = 16.0;
pub const ICON_SLIDER_WIDTH: f32 = 100.0;
pub const ICON_SLIDER_KNOB: u32 = 0x555555ff;
pub const GRID_MIN_CELL: f32 = 64.0;
pub const GRID_CELL_MARGIN: f32 = 16.0;
pub const GRID_CELL_GAP: f32 = 16.0;
pub const GRID_ROW_MARGIN: f32 = 24.0;
pub const GRID_LABEL_PADDING: f32 = 12.0;
pub const GRID_LABEL_SIDE: f32 = 2.0;
pub const GRID_CELL_PADDING: f32 = 4.0;
pub const SETTINGS_WINDOW_WIDTH: f32 = 560.0;
pub const SETTINGS_WINDOW_HEIGHT: f32 = 620.0;
pub const SETTINGS_LABEL_WIDTH: f32 = 190.0;
pub const SETTINGS_SLIDER_WIDTH: f32 = 180.0;
pub const RENAME_SIDE: f32 = 4.0;
pub const BAND_SCROLL_SPEED: f32 = 0.5;
pub const BAND_SCROLL_LIMIT: f32 = 60.0;
pub const SELECTED_SOFT: u32 = 0xcfe2fa;
pub const SCROLLBAR_LINGER_MS: u64 = 1200;
pub const SCROLLBAR_FADE_IN_MS: u64 = 150;
pub const SCROLLBAR_FADE_OUT_MS: u64 = 300;
pub const DRAG_BAR_HEIGHT: f32 = 48.0;
pub const DRAG_BAR_SEGMENTS: usize = 4;
pub const DRAG_SEGMENT_MAX_WIDTH: f32 = 280.0;
pub const GRID_LABEL_LINE: f32 = 16.0;
pub const GRID_LABEL_LINES: f32 = 3.0;
pub const GRID_SIDE_PADDING: f32 = 8.0;
pub const MARQUEE_SPEED: f32 = 24.0;
pub const MARQUEE_PAUSE: f32 = 0.5;
pub const PREVIEW_WIDTH: f32 = 260.0;
pub const ROW_ICON: f32 = 16.0;
pub const PREVIEW_ICON: f32 = 128.0;
pub const INDENT: f32 = 18.0;
pub const DISCLOSURE_WIDTH: f32 = 14.0;
pub const GROUP_MENU_WIDTH: f32 = 220.0;
pub const GROUP_MENU_RIGHT: f32 = 390.0;
pub const SEARCH_FIELD_WIDTH: f32 = 200.0;
pub const SEARCH_BUTTON_WIDTH: f32 = 42.0;
pub const SEARCH_BAR_HEIGHT: f32 = 28.0;
pub const NAV_BUTTON_WIDTH: f32 = 36.0;
pub const NAV_BUTTON_HEIGHT: f32 = 28.0;
pub const NAV_GLYPH_SIZE: f32 = 20.0;
pub const NAV_DIVIDER_HEIGHT: f32 = 16.0;
pub const SHORTCUT_BADGE_RATIO: f32 = 0.26;
pub const SHORTCUT_BADGE_MAX: f32 = 28.0;
pub const SHORTCUT_BADGE_MIN: f32 = 10.0;

pub const BACKGROUND: u32 = 0xffffff;
pub const BAR: u32 = 0xf6f6f7;
pub const SIDEBAR: u32 = 0xeef0f3;
pub const LINE: u32 = 0xdcdde0;
pub const TEXT: u32 = 0x1f2023;
pub const SUB_TEXT: u32 = 0x73767c;
pub const DISABLED_TEXT: u32 = 0xb4b6bb;
pub const STRIPE: u32 = 0xf4f5f7;
pub const HOVER: u32 = 0xe4e6ea;
pub const SELECTED: u32 = 0x0a64d8;
pub const SELECTED_TEXT: u32 = 0xffffff;
pub const SELECTED_DROP: u32 = 0x0848a0;
pub const SELECTED_INACTIVE: u32 = 0xd3d5da;
pub const ERROR_TEXT: u32 = 0xc42b1c;
pub const SEGMENT: u32 = 0xdfe1e5;
pub const FOLDER: u32 = 0x4aa3e8;
pub const FILE: u32 = 0xb8bbc1;
pub const SHORTCUT_ARROW: u32 = 0x1f2023;

/// アイコン表示の名前の折り返し幅。
pub fn grid_label_width(icon_size: f32) -> f32 {
    icon_size.max(GRID_MIN_CELL) + GRID_CELL_MARGIN - GRID_LABEL_PADDING
}
