//! プレビューと再生の、画面にも再生部品にも依存しない判断。

/// マウスを乗せてから再生を始めるまでの待ち時間（ミリ秒）。Eagleと同じ値。
pub const HOVER_DELAY_MS: u64 = 500;
/// 読み込みの表示を出すまでの待ち時間（ミリ秒）。遅いNASの時だけ見える。
pub const SPINNER_DELAY_MS: u64 = 500;
/// マウスで位置を動かした後、再生に戻るまでの待ち時間（ミリ秒）。
pub const SCRUB_RESUME_MS: u64 = 200;
/// マウスの位置から再生位置を送る間隔の下限（ミリ秒）。
pub const SCRUB_THROTTLE_MS: u64 = 50;

/// プレビューでの見せ方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Image,
    Text,
    Video,
    Audio,
    Pdf,
    /// フォルダの中身をツリーで見せる
    Folder,
    /// Zipの目次をツリーで見せる
    Archive,
    Other,
}

impl PreviewKind {
    /// 再生できる種類か。
    pub fn is_media(self) -> bool {
        matches!(self, Self::Video | Self::Audio)
    }
}

const IMAGES: &[&str] = &["avif", "bmp", "dib", "gif", "heic", "heif", "ico", "jfif", "jpe", "jpeg", "jpg", "png", "tif", "tiff", "webp"];
const VIDEOS: &[&str] = &["3g2", "3gp", "3gp2", "3gpp", "asf", "avi", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "webm", "wmv"];
const AUDIOS: &[&str] = &["aac", "adts", "aif", "aiff", "flac", "m4a", "mp3", "oga", "ogg", "opus", "wav", "wma"];
const TEXTS: &[&str] = &[
    "bat", "c", "cfg", "cmd", "conf", "cpp", "cs", "css", "csv", "go", "h", "hpp", "htm", "html", "ini", "java", "js", "json", "jsx", "log", "lua", "md", "markdown", "php", "pl", "ps1",
    "psm1", "py", "rb", "rs", "sh", "sql", "svg", "toml", "tsv", "tsx", "txt", "vue", "xml", "yaml", "yml",
];

/// 名前の拡張子から見せ方を決める。中身は読まないので、一覧の描画から呼べる。
pub fn preview_kind(name: &str, is_dir: bool) -> PreviewKind {
    if is_dir {
        return PreviewKind::Folder;
    }
    let Some((_, extension)) = name.rsplit_once('.') else { return PreviewKind::Other };
    let extension = extension.to_ascii_lowercase();
    let is = |list: &[&str]| list.contains(&extension.as_str());
    if is(VIDEOS) {
        PreviewKind::Video
    } else if is(AUDIOS) {
        PreviewKind::Audio
    } else if is(IMAGES) {
        PreviewKind::Image
    } else if matches!(extension.as_str(), "zip" | "cbz" | "jar") {
        PreviewKind::Archive
    } else if extension == "pdf" {
        PreviewKind::Pdf
    } else if is(TEXTS) {
        PreviewKind::Text
    } else {
        PreviewKind::Other
    }
}

/// 縦横比を保って枠に収める大きさ。元より大きくはしない。
pub fn fit_inside(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if width == 0 || height == 0 || max_width == 0 || max_height == 0 {
        return (0, 0);
    }
    let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64).min(1.);
    (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1))
}

/// マウスオーバーのポップアップの長辺。アイコンより少し大きく、小さなアイコンでも見える大きさにする。
const POPUP_SCALE: f32 = 1.6;
const POPUP_MIN: f32 = 280.;
/// ポップアップの長辺が、ウィンドウの短辺に占める割合の上限。
const POPUP_LIMIT: f32 = 0.9;

/// マウスオーバーのポップアップの大きさ。`aspect` は横に対する縦の比。
pub fn popup_size(aspect: f32, icon: f32, viewport_width: f32, viewport_height: f32) -> (f32, f32) {
    let long = (icon * POPUP_SCALE).max(POPUP_MIN).min(viewport_width.min(viewport_height) * POPUP_LIMIT);
    if aspect > 1. { (long / aspect, long) } else { (long, long * aspect) }
}

/// 中身の縦横比に合わせたウィンドウの中身の大きさ。枠に収めたうえで、小さな中身は枠まで広げる。
/// `extra` は操作の列など、絵の外に要る高さ。
pub fn fit_window(width: f32, height: f32, extra: f32, max_width: f32, max_height: f32) -> (f32, f32) {
    if width <= 0. || height <= 0. {
        return (max_width, max_height);
    }
    let scale = (max_width / width).min((max_height - extra) / height);
    ((width * scale).round(), (height * scale).round() + extra)
}

/// 横の位置を、0から1の割合にする。
pub fn fraction_at(x: f32, left: f32, width: f32) -> f64 {
    if width <= 0. {
        return 0.;
    }
    (((x - left) / width) as f64).clamp(0., 1.)
}

/// 秒を `01:02` や `1:02:03` の形にする。長さに時間の桁があれば、位置にも付ける。
pub fn format_media_time(seconds: f64, duration: f64) -> String {
    let total = if seconds.is_finite() { seconds.max(0.) as u64 } else { 0 };
    let (hours, minutes, rest) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 || (duration.is_finite() && duration >= 3600.) {
        format!("{hours}:{minutes:02}:{rest:02}")
    } else {
        format!("{minutes:02}:{rest:02}")
    }
}

/// マウスを乗せた再生を、今の状態で始めてよいか。
#[derive(Clone, Copy, Debug, Default)]
pub struct HoverGate {
    /// 設定で有効
    pub enabled: bool,
    /// アイコン表示である
    pub icons: bool,
    /// ウィンドウが前面にある
    pub active: bool,
    /// ドラッグ、範囲選択、メニュー、名前の編集のどれかの最中
    pub busy: bool,
    /// クイックルックが開いている
    pub quick_look: bool,
}

impl HoverGate {
    pub fn allows(self) -> bool {
        self.enabled && self.icons && self.active && !self.busy && !self.quick_look
    }
}

/// テキストの先頭のバイト列を文字列にする。読める形でなければ、Shift_JISの解釈に回す。
/// 途中で切ったバイト列を渡してよい。NULを含むなら、テキストではないとみなして `None` を返す。
pub fn decode_text(bytes: &[u8], shift_jis: impl FnOnce(&[u8]) -> Option<String>) -> Option<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(rest).into_owned());
    }
    for (bom, little) in [(&[0xFF, 0xFE], true), (&[0xFE, 0xFF], false)] {
        if let Some(rest) = bytes.strip_prefix(bom) {
            let units: Vec<u16> = rest.chunks_exact(2).map(|c| if little { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
            return Some(String::from_utf16_lossy(&units));
        }
    }
    if bytes.contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_string()),
        // 切った位置で文字が割れただけなら、そこまでを使う
        Err(error) if error.error_len().is_none() => Some(String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned()),
        Err(_) => shift_jis(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_comes_from_the_extension() {
        assert_eq!(preview_kind("a.MP4", false), PreviewKind::Video);
        assert_eq!(preview_kind("a.flac", false), PreviewKind::Audio);
        assert_eq!(preview_kind("a.JPG", false), PreviewKind::Image);
        assert_eq!(preview_kind("a.b.txt", false), PreviewKind::Text);
        assert_eq!(preview_kind("a.PDF", false), PreviewKind::Pdf);
        assert_eq!(preview_kind("a.ZIP", false), PreviewKind::Archive);
        assert_eq!(preview_kind("a.exe", false), PreviewKind::Other);
        assert_eq!(preview_kind("noextension", false), PreviewKind::Other);
    }

    #[test]
    fn a_folder_named_like_a_video_is_not_media() {
        assert_eq!(preview_kind("a.mp4", true), PreviewKind::Folder);
        assert!(!preview_kind("a.mp4", true).is_media());
    }

    #[test]
    fn fitting_keeps_the_ratio_and_never_enlarges() {
        assert_eq!(fit_inside(1920, 1080, 960, 960), (960, 540));
        assert_eq!(fit_inside(1080, 1920, 960, 960), (540, 960));
        assert_eq!(fit_inside(100, 50, 960, 960), (100, 50));
        assert_eq!(fit_inside(0, 50, 960, 960), (0, 0));
        assert_eq!(fit_inside(100, 50, 0, 0), (0, 0));
        assert_eq!(fit_inside(10000, 1, 100, 100), (100, 1));
    }

    #[test]
    fn popup_is_larger_than_the_icon_and_keeps_the_shape() {
        assert_eq!(popup_size(0.5, 96., 1000., 800.), (280., 140.));
        assert_eq!(popup_size(2., 96., 1000., 800.), (140., 280.));
        assert_eq!(popup_size(0.5, 256., 2000., 2000.).0, 256. * 1.6);
    }

    #[test]
    fn popup_stays_inside_a_small_window() {
        let (width, _) = popup_size(0.5, 256., 300., 200.);
        assert_eq!(width, 180.);
    }

    #[test]
    fn window_follows_the_shape_of_the_content() {
        // 縦長は高さで、横長は幅で枠に当たる
        assert_eq!(fit_window(1080., 1920., 0., 960., 720.), (405., 720.));
        assert_eq!(fit_window(1920., 1080., 0., 960., 720.), (960., 540.));
        // 小さな中身も枠まで広げ、操作の列の高さを足す
        assert_eq!(fit_window(160., 90., 32., 960., 720.), (960., 572.));
        assert_eq!(fit_window(0., 0., 32., 960., 720.), (960., 720.));
    }

    #[test]
    fn fraction_stays_inside_the_bar() {
        assert_eq!(fraction_at(50., 0., 100.), 0.5);
        assert_eq!(fraction_at(-10., 0., 100.), 0.);
        assert_eq!(fraction_at(150., 0., 100.), 1.);
        assert_eq!(fraction_at(5., 0., 0.), 0.);
    }

    #[test]
    fn time_uses_hours_only_when_needed() {
        assert_eq!(format_media_time(62., 120.), "01:02");
        assert_eq!(format_media_time(62., 4000.), "0:01:02");
        assert_eq!(format_media_time(3723., 4000.), "1:02:03");
        assert_eq!(format_media_time(f64::NAN, f64::NAN), "00:00");
    }

    #[test]
    fn hover_needs_every_condition() {
        let open = HoverGate { enabled: true, icons: true, active: true, busy: false, quick_look: false };
        assert!(open.allows());
        assert!(!HoverGate { enabled: false, ..open }.allows());
        assert!(!HoverGate { icons: false, ..open }.allows());
        assert!(!HoverGate { active: false, ..open }.allows());
        assert!(!HoverGate { busy: true, ..open }.allows());
        assert!(!HoverGate { quick_look: true, ..open }.allows());
    }

    #[test]
    fn text_is_read_by_its_mark_and_falls_back_to_shift_jis() {
        let never = |_: &[u8]| panic!("not needed");
        assert_eq!(decode_text(b"\xEF\xBB\xBFabc", never).unwrap(), "abc");
        assert_eq!(decode_text(b"\xFF\xFEa\0b\0", never).unwrap(), "ab");
        assert_eq!(decode_text(b"\xFE\xFF\0a\0b", never).unwrap(), "ab");
        assert_eq!(decode_text("日本語".as_bytes(), never).unwrap(), "日本語");
        assert_eq!(decode_text(b"\x93\xFA", |_| Some("日".into())).unwrap(), "日");
    }

    #[test]
    fn a_cut_multibyte_character_does_not_look_like_shift_jis() {
        let bytes = "日本語".as_bytes();
        let text = decode_text(&bytes[..bytes.len() - 1], |_| panic!("not needed")).unwrap();
        assert_eq!(text, "日本");
    }

    #[test]
    fn nul_means_binary() {
        assert!(decode_text(b"ab\0cd", |_| None).is_none());
    }
}
