//! サムネイルをアプリの中で作る。シェルの拡張は通さない。画像はWIC、動画はMedia Foundationで読む。

use crate::icons::encode_png;
use std::io::Read;
use std::path::Path;
use std::sync::Once;
use windows::Win32::Foundation::{E_FAIL, GENERIC_READ};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGR, GUID_WICPixelFormat32bppPBGRA, IWICBitmapFrameDecode, IWICBitmapSource,
    IWICBitmapSourceTransform, IWICImagingFactory, IWICPixelFormatInfo, WICBitmapInterpolationModeFant, WICBitmapTransformRotate0,
    WICConvertBitmapSource, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::Media::MediaFoundation::ICodecAPI;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVDecNumWorkerThreads, CODECAPI_AVDecVideoThumbnailGenerationMode, IMF2DBuffer, IMFAttributes, IMFSample, IMFSourceReader,
    IMFSourceReaderEx, IMFTransform, MF_ACCESSMODE_READ, MF_LOW_LATENCY, MFT_CATEGORY_VIDEO_DECODER, MF_FILEFLAGS_NONE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE,
    MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_PD_DURATION,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
    MF_SOURCE_READER_MEDIASOURCE, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF_VERSION, MFCreateAttributes, MFCreateFile,
    MFCreateMediaType, MFCreateSourceReaderFromByteStream, MFMediaType_Video, MFSTARTUP_FULL, MFStartup, MFVideoFormat_RGB32,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, StructuredStorage::PROPVARIANT};
use windows::core::{GUID, HSTRING, Interface, PCWSTR, Result, w};

/// 真っ黒に見える平均の明るさ（0から255）。
const DARK_LEVEL: u32 = 16;
/// 1回の位置で読むサンプルの上限。絵が出ないファイルで回り続けないため。
const MAX_READS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbKind {
    Image,
    Video,
}

#[derive(Debug)]
pub enum Made {
    /// PNGのバイト列
    Picture(Vec<u8>),
    /// 読めたが絵にできない。ファイルが変わるまで作り直さない
    Unsupported,
    /// ファイルに届かない。後で取り直す
    Unreachable,
    Cancelled,
}

struct Raster {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// 長い辺を `side` に収める大きさ。拡大はしない。
fn fit(width: u32, height: u32, side: u32) -> (u32, u32) {
    let longest = width.max(height).max(1);
    if longest <= side {
        return (width.max(1), height.max(1));
    }
    let scale = |value: u32| ((value as u64 * side as u64 / longest as u64) as u32).max(1);
    (scale(width), scale(height))
}

/// 埋め込みのサムネイルを使ってよいか。小さすぎるものと、黒帯で比率が違うものは使わない。
fn embedded_fits(embedded: (u32, u32), full: (u32, u32), side: u32) -> bool {
    let (ew, eh) = (embedded.0 as u64, embedded.1 as u64);
    let (fw, fh) = (full.0 as u64, full.1 as u64);
    let matches_shape = (ew * fh).abs_diff(eh * fw) * 50 <= ew * fh;
    ew.max(eh) >= side as u64 && matches_shape
}

/// Exifの向き（1から8）を画素へ適用する。
fn orient(raster: Raster, orientation: u16) -> Raster {
    if !(2..=8).contains(&orientation) {
        return raster;
    }
    let (w, h) = (raster.width as usize, raster.height as usize);
    let swaps = orientation >= 5;
    let (out_w, out_h) = if swaps { (h, w) } else { (w, h) };
    let mut rgba = vec![0u8; out_w * out_h * 4];
    for y in 0..h {
        for x in 0..w {
            let (tx, ty) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                _ => (y, w - 1 - x),
            };
            let from = (y * w + x) * 4;
            let to = (ty * out_w + tx) * 4;
            rgba[to..to + 4].copy_from_slice(&raster.rgba[from..from + 4]);
        }
    }
    Raster { width: out_w as u32, height: out_h as u32, rgba }
}

/// 乗算済みのBGRAを、PNGに入れる普通のRGBAへ直す。
fn straight_rgba(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
        pixel.swap(0, 2);
    }
}

fn brightness(rgba: &[u8]) -> u32 {
    let count = (rgba.len() / 4).max(1) as u64;
    let sum: u64 = rgba.chunks_exact(4).map(|p| (p[0] as u64 * 2 + p[1] as u64 * 5 + p[2] as u64) / 8).sum();
    (sum / count) as u32
}

/// 試す位置（100ナノ秒）。先頭は暗転が多いので避ける。長さが分からなければ先頭だけ。
fn frame_positions(duration: u64) -> Vec<i64> {
    if duration == 0 {
        return vec![0];
    }
    [10, 30, 50].iter().map(|percent| (duration / 100 * percent) as i64).collect()
}

/// 上から下の順に、隙間なく詰める。`stride` が負なら下から上に並んだ画素。
fn pack_rows(first_row: *const u8, stride: i32, width: u32, height: u32) -> Vec<u8> {
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as isize {
        // SAFETY: 呼び出し側が、`height` 行それぞれに `row` バイト読めるバッファを渡す
        unsafe { out.extend_from_slice(std::slice::from_raw_parts(first_row.offset(y * stride as isize), row)) };
    }
    out
}

/// ファイルの先頭が読めるか。作れなかった理由が、形式か届かないのかを分ける。
fn reachable(path: &Path) -> bool {
    let mut head = [0u8; 16];
    std::fs::File::open(path).and_then(|mut file| file.read(&mut head)).is_ok()
}

/// サムネイルを作る。呼ぶスレッドはCOMの初期化を済ませておく。
pub fn make_thumbnail(path: &Path, side: u32, kind: ThumbKind, cancelled: &dyn Fn() -> bool) -> Made {
    if cancelled() {
        return Made::Cancelled;
    }
    let made = unsafe {
        match kind {
            ThumbKind::Image => image_thumbnail(path, side).map(Some),
            ThumbKind::Video => video_thumbnail(path, side, cancelled),
        }
    };
    match made {
        Ok(Some(raster)) => encode_png(raster.width, raster.height, &raster.rgba).map_or(Made::Unsupported, Made::Picture),
        Ok(None) => Made::Cancelled,
        Err(_) if reachable(path) => Made::Unsupported,
        Err(_) => Made::Unreachable,
    }
}

unsafe fn wic_factory() -> Result<IWICImagingFactory> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
}

unsafe fn image_thumbnail(path: &Path, side: u32) -> Result<Raster> {
    unsafe {
        let factory = wic_factory()?;
        let name = HSTRING::from(path.as_os_str());
        let decoder = factory.CreateDecoderFromFilename(&name, None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;
        let target = fit(width, height, side);
        let source = small_source(&factory, &frame, (width, height), target, side)?;
        let raster = scale(&factory, &source, target.0, target.1)?;
        Ok(orient(raster, read_orientation(&frame)))
    }
}

/// 埋め込みのサムネイルか、デコーダーが縮小して読んだ絵か、元の絵。軽い順に選ぶ。
unsafe fn small_source(
    factory: &IWICImagingFactory,
    frame: &IWICBitmapFrameDecode,
    full: (u32, u32),
    target: (u32, u32),
    side: u32,
) -> Result<IWICBitmapSource> {
    unsafe {
        if let Ok(embedded) = frame.GetThumbnail() {
            let (mut width, mut height) = (0, 0);
            if embedded.GetSize(&mut width, &mut height).is_ok() && embedded_fits((width, height), full, side) {
                return Ok(embedded);
            }
        }
        if let Some(reduced) = reduced_decode(factory, frame, full, target) {
            return Ok(reduced);
        }
        frame.cast()
    }
}

/// JPEGのように縮小しながら読める形式で、必要な大きさだけ読む。
unsafe fn reduced_decode(
    factory: &IWICImagingFactory,
    frame: &IWICBitmapFrameDecode,
    full: (u32, u32),
    target: (u32, u32),
) -> Option<IWICBitmapSource> {
    unsafe {
        let transform: IWICBitmapSourceTransform = frame.cast().ok()?;
        for shift in 0..4 {
            let (mut width, mut height) = (target.0 << shift, target.1 << shift);
            if width >= full.0 || transform.GetClosestSize(&mut width, &mut height).is_err() {
                return None;
            }
            if width < target.0 || height < target.1 {
                continue;
            }
            if width >= full.0 {
                return None;
            }
            let mut format = GUID_WICPixelFormat32bppBGR;
            transform.GetClosestPixelFormat(&mut format).ok()?;
            let info: IWICPixelFormatInfo = factory.CreateComponentInfo(&format).ok()?.cast().ok()?;
            let bits = info.GetBitsPerPixel().ok()?;
            let stride = (width * bits).div_ceil(8).next_multiple_of(4);
            let mut pixels = vec![0u8; stride as usize * height as usize];
            transform
                .CopyPixels(std::ptr::null(), width, height, &format, WICBitmapTransformRotate0, stride, &mut pixels)
                .ok()?;
            return factory.CreateBitmapFromMemory(width, height, &format, stride, &pixels).ok()?.cast().ok();
        }
        None
    }
}

unsafe fn read_orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    unsafe {
        let Ok(reader) = frame.GetMetadataQueryReader() else { return 1 };
        let mut value = PROPVARIANT::default();
        if reader.GetMetadataByName(w!("System.Photo.Orientation"), &mut value).is_err() {
            return 1;
        }
        u16::try_from(&value).unwrap_or(1)
    }
}

unsafe fn scale(factory: &IWICImagingFactory, source: &IWICBitmapSource, width: u32, height: u32) -> Result<Raster> {
    unsafe {
        let converted = WICConvertBitmapSource(&GUID_WICPixelFormat32bppPBGRA, source)?;
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&converted, width, height, WICBitmapInterpolationModeFant)?;
        let stride = width * 4;
        let mut rgba = vec![0u8; stride as usize * height as usize];
        scaler.CopyPixels(std::ptr::null(), stride, &mut rgba)?;
        straight_rgba(&mut rgba);
        Ok(Raster { width, height, rgba })
    }
}

fn mf_startup() {
    static STARTUP: Once = Once::new();
    STARTUP.call_once(|| {
        let _ = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) };
    });
}

fn failure() -> windows::core::Error {
    E_FAIL.into()
}

unsafe fn open_reader(path: &Path) -> Result<IMFSourceReader> {
    unsafe {
        let name = HSTRING::from(path.as_os_str());
        // URLとして渡すと、名前の # や % で失敗する
        let file = MFCreateFile(MF_ACCESSMODE_READ, MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_FILEFLAGS_NONE, PCWSTR(name.as_ptr()))?;
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 2)?;
        let attributes = attributes.ok_or_else(failure)?;
        attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        // 先の絵を溜めず、最初の1枚をすぐ出させる
        attributes.SetUINT32(&MF_LOW_LATENCY, 1)?;
        MFCreateSourceReaderFromByteStream(&file, &attributes)
    }
}

/// デコーダーを1枚のための設定にする。既定は全コアへ広げて先読みするので、1枚だけには無駄が多い。
/// 対応しないデコーダーもあるので、失敗は無視する。
unsafe fn lighten_decoder(reader: &IMFSourceReader, stream: u32) {
    unsafe {
        let Ok(extended) = reader.cast::<IMFSourceReaderEx>() else { return };
        for index in 0..8 {
            let mut category = GUID::zeroed();
            let mut transform: Option<IMFTransform> = None;
            if extended.GetTransformForStream(stream, index, Some(&mut category), &mut transform).is_err() {
                return;
            }
            if category != MFT_CATEGORY_VIDEO_DECODER {
                continue;
            }
            let Some(Ok(api)) = transform.map(|decoder| decoder.cast::<ICodecAPI>()) else { return };
            let _ = api.SetValue(&CODECAPI_AVDecVideoThumbnailGenerationMode, &VARIANT::from(true));
            if api.SetValue(&CODECAPI_AVDecNumWorkerThreads, &VARIANT::from(1i32)).is_err() {
                let _ = api.SetValue(&CODECAPI_AVDecNumWorkerThreads, &VARIANT::from(1u32));
            }
            return;
        }
    }
}

unsafe fn video_thumbnail(path: &Path, side: u32, cancelled: &dyn Fn() -> bool) -> Result<Option<Raster>> {
    unsafe {
        mf_startup();
        let reader = open_reader(path)?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(stream, true)?;
        let output = MFCreateMediaType()?;
        output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader.SetCurrentMediaType(stream, None, &output)?;
        lighten_decoder(&reader, stream);

        let native = reader.GetNativeMediaType(stream, 0)?;
        // 高度な処理を有効にしたリーダーは、回転の情報も適用した絵を返す。ここでは回さない
        let current = reader.GetCurrentMediaType(stream)?;
        let packed = current.GetUINT64(&MF_MT_FRAME_SIZE)?;
        let (width, height) = ((packed >> 32) as u32, packed as u32);
        if width == 0 || height == 0 {
            return Err(failure());
        }
        let stride = current.GetUINT32(&MF_MT_DEFAULT_STRIDE).map_or(-(width as i32 * 4), |value| value as i32);
        let ratio = native.GetUINT64(&MF_MT_PIXEL_ASPECT_RATIO).unwrap_or(1 << 32 | 1);
        let (numerator, denominator) = (((ratio >> 32) as u64).max(1), ((ratio as u32) as u64).max(1));
        let shown_width = (width as u64 * numerator / denominator).min(u32::MAX as u64) as u32;
        let target = fit(shown_width, height, side);

        let factory = wic_factory()?;
        let duration = reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            .ok()
            .and_then(|value| u64::try_from(&value).ok())
            .unwrap_or(0);
        let mut brightest: Option<Raster> = None;
        let mut last_error = None;
        for position in frame_positions(duration) {
            if position > 0 && reader.SetCurrentPosition(&GUID::zeroed(), &PROPVARIANT::from(position)).is_err() {
                continue;
            }
            let frame = match read_frame(&reader, stream, width, height, stride, cancelled) {
                Ok(Some(frame)) => frame,
                Ok(None) => return Ok(None),
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            let bitmap = factory.CreateBitmapFromMemory(width, height, &GUID_WICPixelFormat32bppBGR, width * 4, &frame)?;
            let raster = scale(&factory, &bitmap.cast()?, target.0, target.1)?;
            if brightness(&raster.rgba) > DARK_LEVEL {
                return Ok(Some(raster));
            }
            if brightest.as_ref().is_none_or(|best| brightness(&best.rgba) < brightness(&raster.rgba)) {
                brightest = Some(raster);
            }
        }
        match brightest {
            Some(raster) => Ok(Some(raster)),
            None => Err(last_error.unwrap_or_else(failure)),
        }
    }
}

unsafe fn read_frame(
    reader: &IMFSourceReader,
    stream: u32,
    width: u32,
    height: u32,
    stride: i32,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Vec<u8>>> {
    unsafe {
        for _ in 0..MAX_READS {
            if cancelled() {
                return Ok(None);
            }
            let mut flags = 0u32;
            let mut sample: Option<IMFSample> = None;
            reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 | MF_SOURCE_READERF_ERROR.0) as u32 != 0 {
                return Err(failure());
            }
            if let Some(sample) = sample {
                return copy_sample(&sample, width, height, stride).map(Some);
            }
        }
        Err(failure())
    }
}

unsafe fn copy_sample(sample: &IMFSample, width: u32, height: u32, stride: i32) -> Result<Vec<u8>> {
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer()?;
        if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
            let (mut first, mut actual) = (std::ptr::null_mut(), 0i32);
            two_d.Lock2D(&mut first, &mut actual)?;
            let frame = pack_rows(first, actual, width, height);
            two_d.Unlock2D()?;
            return Ok(frame);
        }
        let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
        buffer.Lock(&mut data, None, Some(&mut length))?;
        let row = width as u64 * 4;
        let needed = stride.unsigned_abs() as u64 * (height as u64 - 1) + row;
        if (length as u64) < needed || (stride.unsigned_abs() as u64) < row {
            let _ = buffer.Unlock();
            return Err(failure());
        }
        let first = if stride < 0 { data.add(stride.unsigned_abs() as usize * (height as usize - 1)) } else { data };
        let frame = pack_rows(first, stride, width, height);
        buffer.Unlock()?;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shrinks_the_long_side_and_never_enlarges() {
        assert_eq!(fit(1920, 1080, 256), (256, 144));
        assert_eq!(fit(1080, 1920, 128), (72, 128));
        assert_eq!(fit(100, 50, 256), (100, 50));
        assert_eq!(fit(10000, 1, 256), (256, 1));
    }

    #[test]
    fn embedded_pictures_must_be_large_enough_and_keep_the_shape() {
        assert!(embedded_fits((160, 120), (4000, 3000), 128));
        assert!(!embedded_fits((160, 120), (4000, 3000), 256));
        // 16:9の写真に付いた、黒帯入りの4:3
        assert!(!embedded_fits((160, 120), (1920, 1080), 128));
    }

    fn sample() -> Raster {
        // 幅2、高さ1。左が赤、右が青
        Raster { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] }
    }

    #[test]
    fn orientation_moves_pixels_where_exif_says() {
        let red_at = |raster: &Raster, x: usize, y: usize| raster.rgba[(y * raster.width as usize + x) * 4] == 255;
        assert!(red_at(&orient(sample(), 1), 0, 0));
        assert!(red_at(&orient(sample(), 2), 1, 0));
        assert!(red_at(&orient(sample(), 3), 1, 0));
        assert!(red_at(&orient(sample(), 4), 0, 0));
        let cases = [(5, 0, 0), (6, 0, 0), (7, 0, 1), (8, 0, 1)];
        for (code, x, y) in cases {
            let rotated = orient(sample(), code);
            assert_eq!((rotated.width, rotated.height), (1, 2));
            assert!(red_at(&rotated, x, y), "orientation {code}");
        }
    }

    #[test]
    fn premultiplied_blue_green_red_becomes_straight_rgba() {
        let mut pixels = vec![10, 20, 64, 128, 1, 2, 3, 255, 9, 9, 9, 0];
        straight_rgba(&mut pixels);
        assert_eq!(pixels, vec![128, 40, 20, 128, 3, 2, 1, 255, 9, 9, 9, 0]);
    }

    #[test]
    fn black_frames_are_dark_and_pictures_are_not() {
        assert!(brightness(&[0, 0, 0, 255].repeat(4)) <= DARK_LEVEL);
        assert!(brightness(&[200, 180, 160, 255].repeat(4)) > DARK_LEVEL);
    }

    #[test]
    fn frames_are_taken_away_from_the_start() {
        assert_eq!(frame_positions(0), vec![0]);
        assert_eq!(frame_positions(1000), vec![100, 300, 500]);
    }

    #[test]
    fn rows_are_packed_top_first_for_both_stride_directions() {
        let source: Vec<u8> = (0..16).collect();
        assert_eq!(pack_rows(source.as_ptr(), 8, 1, 2), vec![0, 1, 2, 3, 8, 9, 10, 11]);
        let bottom_up = unsafe { source.as_ptr().add(8) };
        assert_eq!(pack_rows(bottom_up, -8, 1, 2), vec![8, 9, 10, 11, 0, 1, 2, 3]);
    }
}
