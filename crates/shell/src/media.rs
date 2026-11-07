//! Media Foundationの再生部品と、WICの画像の読み込み。
//!
//! 再生器は1つにつき専用のスレッドで動く。開く、シーク、終了は遅いNASの上で待たされることがあるので、
//! 描画のスレッドからは値を置くだけにする。`Player` を捨てると、スレッドが数ミリ秒で音を止める。

use filer_core::{MAX_TREE_LINES, TreeItem, decode_text, fit_inside, read_zip_items};
use std::collections::VecDeque;
use std::fs::File;
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
use windows::Win32::Foundation::{GENERIC_READ, HMODULE, HWND, RECT, S_OK};
use windows::Win32::Globalization::{MB_ERR_INVALID_CHARS, MULTI_BYTE_TO_WIDE_CHAR_FLAGS, MultiByteToWideChar};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICBitmapTransformFlipHorizontal, WICBitmapTransformFlipVertical, WICBitmapTransformOptions, WICBitmapTransformRotate0, WICBitmapTransformRotate90, WICBitmapTransformRotate180,
    WICBitmapTransformRotate270, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFDXGIDeviceManager, IMFMediaEngine, IMFMediaEngineClassFactory, IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl,
    MF_ACCESSMODE_READ, MF_FILEFLAGS_NONE, MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_DXGI_MANAGER, MF_MEDIA_ENGINE_EVENT_ERROR, MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE,
    MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_VERSION, MFARGB, MFCreateAttributes, MFCreateDXGIDeviceManager, MFCreateFile, MFSTARTUP_FULL, MFStartup,
};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::UI::WindowsAndMessaging::{GWLP_HWNDPARENT, GetClientRect, GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos};
use windows::core::{BSTR, HSTRING, Interface, PCWSTR, implement};

/// 映像の検査の間隔。60fpsの1コマより短くする。
const VIDEO_TICK: Duration = Duration::from_millis(4);
/// 音声だけの時や開いている途中の検査の間隔。止める指示もこの間隔で受ける。
const IDLE_TICK: Duration = Duration::from_millis(15);

/// BGRAの1枚の絵。透明の扱いは事前乗算。
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// 再生の様子。描画はこれを読むだけにする。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerStatus {
    /// 長さが分かり、再生を始められる
    pub ready: bool,
    pub failed: bool,
    pub paused: bool,
    pub ended: bool,
    pub has_video: bool,
    /// 映像の元の大きさ
    pub video_size: (u32, u32),
    pub position: f64,
    pub duration: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct PlayerOptions {
    pub looping: bool,
    pub muted: bool,
    pub autoplay: bool,
}

/// 描画のスレッドから置く指示と、再生のスレッドから置く結果。
#[derive(Default)]
struct Board {
    status: PlayerStatus,
    frame: Option<Picture>,
    /// まだ渡していない最新のシーク先（秒）
    seek: Option<f64>,
    /// 再生か一時停止の指示
    play: Option<bool>,
    muted: Option<bool>,
    /// 映像を出す枠の大きさ（物理ピクセル）
    target: (u32, u32),
}

struct Shared {
    stop: AtomicBool,
    failed: AtomicBool,
    board: Mutex<Board>,
}

/// 再生器の窓口。捨てると止まる。
pub struct Player {
    shared: Arc<Shared>,
}

impl Player {
    /// 別のスレッドで開き始める。ここでは待たない。`wake` は新しい絵が届くたびに再生のスレッドから呼ぶ。
    pub fn open(path: PathBuf, options: PlayerOptions, wake: impl Fn() + Send + 'static) -> Self {
        static STARTUP: Once = Once::new();
        let shared = Arc::new(Shared { stop: AtomicBool::new(false), failed: AtomicBool::new(false), board: Mutex::new(Board::default()) });
        let worker = shared.clone();
        let spawned = std::thread::Builder::new().name("media".into()).spawn(move || unsafe {
            let initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            STARTUP.call_once(|| {
                let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
            });
            if run_engine(&path, options, &worker, &wake).is_err() {
                worker.failed.store(true, Ordering::SeqCst);
                worker.board().status.failed = true;
            }
            if initialized {
                CoUninitialize();
            }
        });
        if spawned.is_err() {
            shared.board().status.failed = true;
        }
        Self { shared }
    }

    /// 今の様子。
    pub fn status(&self) -> PlayerStatus {
        self.shared.board().status
    }

    /// 前回から新しい絵があれば受け取る。
    pub fn take_frame(&self) -> Option<Picture> {
        self.shared.board().frame.take()
    }

    pub fn play(&self, playing: bool) {
        self.shared.board().play = Some(playing);
    }

    /// 秒で位置を指定する。送りきれない分は最新だけが残る。
    pub fn seek(&self, seconds: f64) {
        self.shared.board().seek = Some(seconds);
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.board().muted = Some(muted);
    }

    /// 絵の大きさの上限を伝える。小さな枠に大きな絵を運ばないためのもの。
    pub fn set_target(&self, width: u32, height: u32) {
        self.shared.board().target = (width, height);
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

impl Shared {
    fn board(&self) -> std::sync::MutexGuard<'_, Board> {
        self.board.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
}

#[implement(IMFMediaEngineNotify)]
struct Notify {
    shared: Arc<Shared>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, _param1: usize, _param2: u32) -> windows::core::Result<()> {
        // 様子は再生のスレッドが見に行く。失敗だけは、問い合わせでは分からない
        if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            self.shared.failed.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// 拡大縮小の受け皿。読み戻すために、同じ大きさの作業用をもう1枚持つ。
struct Target {
    width: u32,
    height: u32,
    render: ID3D11Texture2D,
    staging: ID3D11Texture2D,
}

fn texture_desc(width: u32, height: u32) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    }
}

unsafe fn make_target(device: &ID3D11Device, width: u32, height: u32) -> windows::core::Result<Target> {
    let desc = texture_desc(width, height);
    let staging_desc = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, ..desc };
    let (mut render, mut staging) = (None, None);
    unsafe {
        device.CreateTexture2D(&desc, None, Some(&mut render))?;
        device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
    }
    Ok(Target { width, height, render: render.unwrap(), staging: staging.unwrap() })
}

/// 動画のための部品を作る。映像の無いファイルでも同じ手順で通る。
unsafe fn create_engine(shared: &Arc<Shared>) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext, IMFMediaEngine)> {
    unsafe {
        let (mut device, mut context) = (None, None);
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
        let (device, context): (ID3D11Device, ID3D11DeviceContext) = (device.unwrap(), context.unwrap());
        // 部品の内部のスレッドと、このスレッドの読み戻しが同じ装置を使う
        let _ = device.cast::<ID3D10Multithread>()?.SetMultithreadProtected(true);
        let mut token = 0;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
        let manager = manager.unwrap();
        manager.ResetDevice(&device, token)?;
        let mut attributes: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributes, 3)?;
        let attributes = attributes.unwrap();
        let notify: IMFMediaEngineNotify = Notify { shared: shared.clone() }.into();
        attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
        attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
        attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32)?;
        let factory: IMFMediaEngineClassFactory = CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_ALL)?;
        let engine = factory.CreateInstance(0, &attributes)?;
        Ok((device, context, engine))
    }
}

/// 消音は音量0で表す。`SetMuted` は読み込みの前に指定すると効かない。
fn volume_of(muted: bool) -> f64 {
    if muted { 0. } else { 1. }
}

/// 新しいコマがあればその時刻を返す。
/// `OnVideoStreamTick` の包みは「新しいコマが無い」の結果を返さないので、元の結果を見る。
unsafe fn next_frame(engine: &IMFMediaEngine) -> Option<i64> {
    let mut time = 0i64;
    let result = unsafe { (Interface::vtable(engine).OnVideoStreamTick)(Interface::as_raw(engine), &mut time) };
    (result == S_OK).then_some(time)
}

/// 今のコマを枠の大きさの絵にする。
unsafe fn grab(engine: &IMFMediaEngine, context: &ID3D11DeviceContext, target: &Target) -> windows::core::Result<Picture> {
    let rect = RECT { left: 0, top: 0, right: target.width as i32, bottom: target.height as i32 };
    let border = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
    unsafe {
        engine.TransferVideoFrame(&target.render, None, &rect, Some(&border))?;
        context.CopyResource(&target.staging, &target.render);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&target.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        let row = target.width as usize * 4;
        let mut bgra = vec![0u8; row * target.height as usize];
        for y in 0..target.height as usize {
            let source = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
            std::ptr::copy_nonoverlapping(source, bgra.as_mut_ptr().add(y * row), row);
        }
        context.Unmap(&target.staging, 0);
        Ok(Picture { width: target.width, height: target.height, bgra })
    }
}

fn run_engine(path: &Path, options: PlayerOptions, shared: &Arc<Shared>, wake: &dyn Fn()) -> windows::core::Result<()> {
    unsafe {
        let (device, context, engine) = create_engine(shared)?;
        let result = drive(path, options, shared, &device, &context, &engine, wake);
        // 音を先に消す。Shutdownは遅いNASの上で待たされることがある
        let _ = engine.SetVolume(0.);
        let _ = engine.Pause();
        let _ = engine.Shutdown();
        result
    }
}

unsafe fn drive(
    path: &Path,
    options: PlayerOptions,
    shared: &Arc<Shared>,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    engine: &IMFMediaEngine,
    wake: &dyn Fn(),
) -> windows::core::Result<()> {
    unsafe {
        engine.SetVolume(volume_of(options.muted))?;
        engine.SetLoop(options.looping)?;
        engine.SetAutoPlay(options.autoplay)?;
        if shared.stopped() {
            return Ok(());
        }
        // URLの形にしないので、名前の # や % をそのまま扱える
        let wide = HSTRING::from(path.as_os_str());
        let stream = MFCreateFile(MF_ACCESSMODE_READ, MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_FILEFLAGS_NONE, PCWSTR(wide.as_ptr()))?;
        if shared.stopped() {
            return Ok(());
        }
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let extended: IMFMediaEngineEx = engine.cast()?;
        extended.SetSourceFromByteStream(&stream, &BSTR::from(name))?;
        if options.autoplay {
            // 属性の自動再生だけでは、映像の出先が無いこの使い方では始まらない
            let _ = engine.Play();
        }
        let mut target: Option<Target> = None;
        let mut size = (0, 0);
        let mut last_time = None;
        while !shared.stopped() {
            if shared.failed.load(Ordering::SeqCst) {
                return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
            }
            let ready = engine.GetReadyState() >= 1;
            let has_video = ready && engine.HasVideo().as_bool();
            let (play, seek, muted, wanted) = {
                let mut board = shared.board();
                (board.play.take(), if ready && !engine.IsSeeking().as_bool() { board.seek.take() } else { None }, board.muted.take(), board.target)
            };
            if let Some(muted) = muted {
                let _ = engine.SetVolume(volume_of(muted));
            }
            if let Some(seconds) = seek {
                let _ = extended.SetCurrentTimeEx(seconds, MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE);
            }
            match play {
                Some(true) => {
                    let _ = engine.Play();
                }
                Some(false) => {
                    let _ = engine.Pause();
                }
                None => {}
            }
            let mut frame = None;
            if has_video {
                if size == (0, 0) {
                    let (mut width, mut height) = (0, 0);
                    if engine.GetNativeVideoSize(Some(&mut width), Some(&mut height)).is_ok() {
                        size = (width, height);
                    }
                }
                if let Some(time) = next_frame(engine) {
                    let (width, height) = fit_inside(size.0, size.1, wanted.0, wanted.1);
                    let fresh = last_time != Some(time) || target.as_ref().is_some_and(|t| (t.width, t.height) != (width, height));
                    if fresh && width > 0 {
                        if target.as_ref().is_none_or(|t| (t.width, t.height) != (width, height)) {
                            target = Some(make_target(device, width, height)?);
                        }
                        frame = target.as_ref().and_then(|t| grab(engine, context, t).ok());
                        last_time = Some(time);
                    }
                }
            }
            let duration = engine.GetDuration();
            let status = PlayerStatus {
                ready,
                failed: false,
                paused: engine.IsPaused().as_bool(),
                ended: engine.IsEnded().as_bool(),
                has_video,
                video_size: size,
                position: engine.GetCurrentTime(),
                duration: if duration.is_finite() { duration } else { 0. },
            };
            let fresh = frame.is_some();
            {
                let mut board = shared.board();
                board.status = status;
                if fresh {
                    board.frame = frame;
                }
            }
            if fresh {
                wake();
            }
            std::thread::sleep(if has_video { VIDEO_TICK } else { IDLE_TICK });
        }
        Ok(())
    }
}

/// EXIFの向きの値から、まっすぐにする変換を引く。
fn orientation_transform(orientation: u16) -> WICBitmapTransformOptions {
    let flip = WICBitmapTransformFlipHorizontal.0;
    match orientation {
        2 => WICBitmapTransformFlipHorizontal,
        3 => WICBitmapTransformRotate180,
        4 => WICBitmapTransformFlipVertical,
        5 => WICBitmapTransformOptions(WICBitmapTransformRotate90.0 | flip),
        6 => WICBitmapTransformRotate90,
        7 => WICBitmapTransformOptions(WICBitmapTransformRotate270.0 | flip),
        8 => WICBitmapTransformRotate270,
        _ => WICBitmapTransformRotate0,
    }
}

/// 画像を枠に収まる大きさで読む。Windowsに入っている形式はすべて読める。止まらないよう別のスレッドから呼ぶ。
pub fn decode_image(path: &Path, max_width: u32, max_height: u32) -> windows::core::Result<Picture> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_ALL)?;
        let wide = HSTRING::from(path.as_os_str());
        let decoder = factory.CreateDecoderFromFilename(&wide, None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;
        let orientation = frame
            .GetMetadataQueryReader()
            .ok()
            .and_then(|reader| {
                let mut value = windows::Win32::System::Com::StructuredStorage::PROPVARIANT::default();
                reader.GetMetadataByName(windows::core::w!("/app1/ifd/{ushort=274}"), &mut value).ok()?;
                u16::try_from(&value).ok()
            })
            .unwrap_or(1);
        let mut source: IWICBitmapSource = frame.cast()?;
        let turned = (5..=8).contains(&orientation);
        // 向きを直した後の縦横で枠に収める
        let (shown_width, shown_height) = if turned { (height, width) } else { (width, height) };
        let (fit_width, fit_height) = fit_inside(shown_width, shown_height, max_width, max_height);
        let (scaled_width, scaled_height) = if turned { (fit_height, fit_width) } else { (fit_width, fit_height) };
        if (scaled_width, scaled_height) != (width, height) && scaled_width > 0 {
            let scaler = factory.CreateBitmapScaler()?;
            scaler.Initialize(&source, scaled_width, scaled_height, WICBitmapInterpolationModeFant)?;
            source = scaler.cast()?;
        }
        if orientation != 1 {
            let rotator = factory.CreateBitmapFlipRotator()?;
            rotator.Initialize(&source, orientation_transform(orientation))?;
            source = rotator.cast()?;
        }
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(&source, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0., WICBitmapPaletteTypeCustom)?;
        let (mut width, mut height) = (0, 0);
        converter.GetSize(&mut width, &mut height)?;
        let mut bgra = vec![0u8; width as usize * height as usize * 4];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut bgra)?;
        Ok(Picture { width, height, bgra })
    }
}

/// 開いたPDF。ページの絵は必要な時に作る。
#[derive(Clone)]
pub struct Pdf {
    document: PdfDocument,
    pub pages: u32,
}

impl Pdf {
    /// PDFを開く。パスワードが要るものや壊れたものはエラーになる。止まらないよう別のスレッドから呼ぶ。
    pub fn open(path: &Path) -> windows::core::Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.get()?;
        let document = PdfDocument::LoadFromFileAsync(&file)?.get()?;
        let pages = document.PageCount()?;
        Ok(Self { document, pages })
    }

    /// 1ページを枠に収まる大きさの絵にする。小さなページは枠まで拡大する。止まらないよう別のスレッドから呼ぶ。
    pub fn render(&self, index: u32, max_width: u32, max_height: u32) -> windows::core::Result<Picture> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let page = self.document.GetPage(index)?;
        let size = page.Size()?;
        // 枠より大きな仮の大きさから縮めることで、拡大も同じ計算にする
        let huge = (max_width.max(max_height) as f32 * 8.).max(1.) / size.Width.max(size.Height).max(1.);
        let (width, height) = fit_inside((size.Width * huge) as u32, (size.Height * huge) as u32, max_width, max_height);
        let options = PdfPageRenderOptions::new()?;
        options.SetDestinationWidth(width)?;
        options.SetDestinationHeight(height)?;
        let stream = InMemoryRandomAccessStream::new()?;
        page.RenderWithOptionsToStreamAsync(&stream, &options)?.get()?;
        let length = stream.Size()? as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        reader.LoadAsync(length)?.get()?;
        let mut encoded = vec![0u8; length as usize];
        reader.ReadBytes(&mut encoded)?;
        decode_memory(&encoded)
    }
}

/// メモリ上の画像を読む。
fn decode_memory(bytes: &[u8]) -> windows::core::Result<Picture> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_ALL)?;
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0., WICBitmapPaletteTypeCustom)?;
        let (mut width, mut height) = (0, 0);
        converter.GetSize(&mut width, &mut height)?;
        let mut bgra = vec![0u8; width as usize * height as usize * 4];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut bgra)?;
        Ok(Picture { width, height, bgra })
    }
}

/// ウィンドウの中身を指定の大きさ（物理ピクセル）にする。中心は保ち、画面の作業領域から出さない。
/// メッセージが回るので、gpuiの更新処理の外で呼ぶ。
pub fn fit_window_client(hwnd: isize, width: i32, height: i32) {
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        let (mut outer, mut client) = (RECT::default(), RECT::default());
        if GetWindowRect(hwnd, &mut outer).is_err() || GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
        let total_width = width + (outer.right - outer.left) - client.right;
        let total_height = height + (outer.bottom - outer.top) - client.bottom;
        let mut left = (outer.left + outer.right - total_width) / 2;
        let mut top = (outer.top + outer.bottom - total_height) / 2;
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            let area = info.rcWork;
            left = left.min(area.right - total_width).max(area.left);
            top = top.min(area.bottom - total_height).max(area.top);
        }
        let _ = SetWindowPos(hwnd, None, left, top, total_width, total_height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// `child` を `owner` の持ち物にする。`owner` より前に出続け、一緒に最小化される。
pub fn set_window_owner(child: isize, owner: isize) {
    unsafe {
        SetWindowLongPtrW(HWND(child as *mut _), GWLP_HWNDPARENT, owner);
    }
}

/// Zipの目次を読む。止まらないよう別のスレッドから呼ぶ。
pub fn read_zip(path: &Path) -> std::io::Result<(Vec<TreeItem>, bool)> {
    read_zip_items(&mut File::open(path)?, decode_shift_jis)
}

/// フォルダの配下を、浅い階層から順に読む。上限か時間切れで打ち切ると真も返す。止まらないよう別のスレッドから呼ぶ。
pub fn walk_folder(root: &Path, show_hidden: bool, budget: Duration) -> (Vec<TreeItem>, bool) {
    const HIDDEN: u32 = 0x2;
    let start = Instant::now();
    let mut items = Vec::new();
    let mut waiting = VecDeque::from([(root.to_path_buf(), String::new())]);
    while let Some((folder, prefix)) = waiting.pop_front() {
        // 読めないフォルダは空として扱う
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            if items.len() >= MAX_TREE_LINES || start.elapsed() > budget {
                return (items, true);
            }
            // 一覧の読み取りで分かる値だけを使うので、項目ごとにディスクへ行かない
            let Ok(meta) = entry.metadata() else { continue };
            if !show_hidden && meta.file_attributes() & HIDDEN != 0 {
                continue;
            }
            let path = format!("{prefix}{}", entry.file_name().to_string_lossy());
            let is_dir = meta.is_dir();
            // リンクの先は辿らない。自分を指すリンクで終わらなくなる
            if is_dir && !meta.file_type().is_symlink() {
                waiting.push_back((entry.path(), format!("{path}/")));
            }
            items.push(TreeItem { path, is_dir, size: if is_dir { 0 } else { meta.len() } });
        }
    }
    (items, false)
}

/// Shift_JISとして読む。読めないバイトがあれば `None`。
fn decode_shift_jis(bytes: &[u8]) -> Option<String> {
    const SHIFT_JIS: u32 = 932;
    // 末尾の1バイトが2バイト文字の前半で切れていても読めるように、1度だけ削る
    for trimmed in [bytes, &bytes[..bytes.len().saturating_sub(1)]] {
        unsafe {
            let flags: MULTI_BYTE_TO_WIDE_CHAR_FLAGS = MB_ERR_INVALID_CHARS;
            let length = MultiByteToWideChar(SHIFT_JIS, flags, trimmed, None);
            if length <= 0 {
                continue;
            }
            let mut wide = vec![0u16; length as usize];
            MultiByteToWideChar(SHIFT_JIS, flags, trimmed, Some(&mut wide));
            return Some(String::from_utf16_lossy(&wide));
        }
    }
    None
}

/// テキストの先頭を読む。テキストでなければ `None`。止まらないよう別のスレッドから呼ぶ。
pub fn read_text_head(path: &Path, limit: usize) -> Option<String> {
    let mut bytes = Vec::with_capacity(limit);
    File::open(path).ok()?.take(limit as u64).read_to_end(&mut bytes).ok()?;
    decode_text(&bytes, decode_shift_jis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(player: &Player, mut done: impl FnMut(&PlayerStatus) -> bool) -> PlayerStatus {
        let start = Instant::now();
        loop {
            let status = player.status();
            if done(&status) || start.elapsed() > Duration::from_secs(10) {
                return status;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// 無音のPCMのWAVを作る。
    fn wav(seconds: u32) -> Vec<u8> {
        let (rate, data_len) = (8000u32, 8000 * seconds);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(bytes.len() + data_len as usize, 128);
        bytes
    }

    const OPTIONS: PlayerOptions = PlayerOptions { looping: false, muted: true, autoplay: false };

    #[test]
    fn missing_file_fails_without_panicking() {
        let player = Player::open("Z:/no/such/file.mp4".into(), OPTIONS, || {});
        assert!(wait_for(&player, |status| status.failed).failed);
    }

    #[test]
    fn a_broken_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.mp4");
        std::fs::write(&path, b"this is not a movie").unwrap();
        let player = Player::open(path, OPTIONS, || {});
        assert!(wait_for(&player, |status| status.failed).failed);
    }

    #[test]
    fn a_name_with_url_characters_opens_and_reports_its_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("音 #1 100%.wav");
        std::fs::write(&path, wav(2)).unwrap();
        let player = Player::open(path, OPTIONS, || {});
        let status = wait_for(&player, |status| status.ready || status.failed);
        assert!(status.ready, "{status:?}");
        assert!(!status.has_video);
        assert!((status.duration - 2.).abs() < 0.1, "{status:?}");
    }

    #[test]
    fn seek_moves_the_position() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, wav(4)).unwrap();
        let player = Player::open(path, OPTIONS, || {});
        wait_for(&player, |status| status.ready);
        player.seek(2.);
        let status = wait_for(&player, |status| status.position > 1.5);
        assert!(status.position > 1.5, "{status:?}");
    }

    #[test]
    fn dropping_while_opening_does_not_hang_or_panic() {
        for _ in 0..5 {
            drop(Player::open("Z:/no/such/file.mp4".into(), OPTIONS, || {}));
        }
    }

    #[test]
    fn orientation_turns_only_when_marked() {
        assert_eq!(orientation_transform(1), WICBitmapTransformRotate0);
        assert_eq!(orientation_transform(6), WICBitmapTransformRotate90);
        assert_eq!(orientation_transform(8), WICBitmapTransformRotate270);
    }

    #[test]
    fn text_head_reads_shift_jis_and_refuses_binary() {
        let dir = tempfile::tempdir().unwrap();
        let sjis = dir.path().join("s.txt");
        std::fs::write(&sjis, b"\x93\xFA\x96\x7B\x8C\xEA").unwrap();
        assert_eq!(read_text_head(&sjis, 1024).unwrap(), "日本語");
        // 2バイト文字の途中で切れても読める
        assert_eq!(read_text_head(&sjis, 5).unwrap(), "日本");
        let binary = dir.path().join("b.bin");
        std::fs::write(&binary, [0u8, 1, 2]).unwrap();
        assert!(read_text_head(&binary, 1024).is_none());
    }

    /// 白紙2ページのPDFを、参照表まで含めて作る。
    fn blank_pdf() -> Vec<u8> {
        let objects = [
            "<</Type/Catalog/Pages 2 0 R>>",
            "<</Type/Pages/Kids[3 0 R 4 0 R]/Count 2>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 400]>>",
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 400 200]>>",
        ];
        let mut text = String::from("%PDF-1.4
");
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(text.len());
            text.push_str(&format!("{} 0 obj
{object}
endobj
", index + 1));
        }
        let table = text.len();
        text.push_str(&format!("xref
0 {}
0000000000 65535 f 
", objects.len() + 1));
        for offset in offsets {
            text.push_str(&format!("{offset:010} 00000 n 
"));
        }
        text.push_str(&format!("trailer
<</Size {}/Root 1 0 R>>
startxref
{table}
%%EOF
", objects.len() + 1));
        text.into_bytes()
    }

    #[test]
    fn pdf_pages_fit_the_box_and_keep_their_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("資料 #1.pdf");
        std::fs::write(&path, blank_pdf()).unwrap();
        let pdf = Pdf::open(&path).unwrap();
        assert_eq!(pdf.pages, 2);
        let tall = pdf.render(0, 800, 800).unwrap();
        assert_eq!((tall.width, tall.height), (400, 800));
        assert_eq!(tall.bgra.len(), 400 * 800 * 4);
        // 白紙は白で描かれる
        assert_eq!(&tall.bgra[..4], &[255, 255, 255, 255]);
        let wide = pdf.render(1, 800, 800).unwrap();
        assert_eq!((wide.width, wide.height), (800, 400));
        assert!(pdf.render(5, 800, 800).is_err());
    }

    #[test]
    fn broken_pdf_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"x").unwrap();
        assert!(Pdf::open(&path).is_err());
    }

    #[test]
    fn walking_finds_nested_items_and_skips_hidden_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/c.txt"), b"123").unwrap();
        let hidden = dir.path().join("h.txt");
        std::fs::write(&hidden, b"1").unwrap();
        let wide = HSTRING::from(hidden.as_os_str());
        unsafe { windows::Win32::Storage::FileSystem::SetFileAttributesW(&wide, windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN).unwrap() };
        let (mut items, cut) = walk_folder(dir.path(), false, Duration::from_secs(5));
        items.sort_by(|a, b| a.path.cmp(&b.path));
        assert!(!cut);
        let paths: Vec<&str> = items.iter().map(|item| item.path.as_str()).collect();
        assert_eq!(paths, ["a", "a/b", "a/b/c.txt"]);
        assert_eq!(items[2].size, 3);
        let (items, _) = walk_folder(dir.path(), true, Duration::from_secs(5));
        assert_eq!(items.len(), 4);
    }

    #[test]
    fn walking_stops_when_time_runs_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"1").unwrap();
        let (items, cut) = walk_folder(dir.path(), true, Duration::ZERO);
        assert!(cut);
        assert!(items.is_empty());
    }

    #[test]
    fn walking_a_missing_folder_is_empty() {
        let (items, cut) = walk_folder(Path::new("Z:/no/such/folder"), true, Duration::from_secs(1));
        assert!(items.is_empty() && !cut);
    }

    #[test]
    fn broken_image_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, b"not an image").unwrap();
        assert!(decode_image(&path, 100, 100).is_err());
    }
}
