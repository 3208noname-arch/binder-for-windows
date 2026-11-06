use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::GetLogicalDriveStringsW;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Pictures, FOLDERID_Profile,
    IShellItemImageFactory, KF_FLAG_DEFAULT, SHCreateItemFromParsingName, SHGetKnownFolderPath,
    SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::core::{GUID, HSTRING};

/// 裏のスレッドでシェルの部品を使う前に、1回呼ぶ。
pub fn init_thread() {
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
}

/// ファイルのアイコンかサムネイルをPNGで返す。`thumbnail` が偽ならアイコンだけを取る。
pub fn load_icon(path: &Path, size: u32, thumbnail: bool) -> Option<Vec<u8>> {
    let side = size as i32;
    let flags = if thumbnail { SIIGBF_BIGGERSIZEOK } else { SIIGBF_ICONONLY };
    let bitmap = unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        factory.GetImage(SIZE { cx: side, cy: side }, flags).ok()?
    };
    let pixels = read_bitmap(bitmap);
    let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
    let (width, height, rgba) = pixels?;
    encode_png(width, height, &rgba)
}

/// 32ビットの画像をRGBAで読む。
fn read_bitmap(bitmap: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    let mut info = BITMAP::default();
    let read = unsafe {
        GetObjectW(HGDIOBJ(bitmap.0), size_of::<BITMAP>() as i32, Some(&mut info as *mut _ as *mut _))
    };
    if read == 0 || info.bmWidth <= 0 || info.bmHeight <= 0 {
        return None;
    }
    let (width, height) = (info.bmWidth as u32, info.bmHeight as u32);
    let mut header = BITMAPINFO::default();
    header.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width as i32,
        biHeight: -(height as i32), // 上から下の順で受ける
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let mut bgra = vec![0u8; width as usize * height as usize * 4];
    let copied = unsafe {
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, bitmap, 0, height, Some(bgra.as_mut_ptr().cast()), &mut header, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        lines
    };
    if copied == 0 {
        return None;
    }
    // サムネイルは透明度が全部0で返ることがあり、そのままだと見えない
    let opaque = bgra.chunks_exact(4).all(|p| p[3] == 0);
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if opaque {
            pixel[3] = 255;
        }
    }
    Some((width, height, bgra))
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.write_header().ok()?.write_image_data(rgba).ok()?;
    Some(bytes)
}

/// サイドバーのドライブ一覧。
pub fn drives() -> Vec<PathBuf> {
    let mut buffer = [0u16; 256];
    let len = unsafe { GetLogicalDriveStringsW(Some(&mut buffer)) } as usize;
    buffer[..len.min(buffer.len())]
        .split(|&c| c == 0)
        .filter(|drive| !drive.is_empty())
        .map(|drive| OsString::from_wide(drive).into())
        .collect()
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let text = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = OsString::from_wide(text.as_wide()).into();
        CoTaskMemFree(Some(text.0 as *const _));
        Some(path)
    }
}

/// サイドバーのよく使う項目。
pub fn favorites() -> Vec<(String, PathBuf)> {
    [
        ("ホーム", FOLDERID_Profile),
        ("デスクトップ", FOLDERID_Desktop),
        ("ダウンロード", FOLDERID_Downloads),
        ("書類", FOLDERID_Documents),
        ("ピクチャ", FOLDERID_Pictures),
    ]
    .iter()
    .filter_map(|(name, id)| Some((name.to_string(), known_folder(id)?)))
    .collect()
}
