use std::ffi::{OsString, c_void};
use std::path::Path;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleDC, CreateDIBSection,
    CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DeleteDC, DeleteObject, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DoDragDrop, IDropSource, IDropSource_Impl};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_EXPLORE, CMF_NORMAL, CMINVOKECOMMANDINFO, DefSubclassProc, GCS_VERBA, IContextMenu,
    IContextMenu2, IContextMenu3, IDragSourceHelper, ILFindLastID, IShellFolder, RemoveWindowSubclass, SHBindToParent,
    SHDRAGIMAGE, SHGetDesktopFolder, SHParseDisplayName, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, SW_SHOWNORMAL, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};
use windows::core::{BOOL, GUID, HRESULT, HSTRING, Interface, PCSTR, PSTR, Result, implement, w};
use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};

const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7fff;
const CLSID_DRAG_DROP_HELPER: GUID = GUID::from_u128(0x4657278a_411b_11d2_839a_00c04fd918d0);

fn to_hwnd(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

/// パスから `ITEMIDLIST` を作る。使い終わったら `CoTaskMemFree` で解放する。
struct Pidl(*mut ITEMIDLIST);

impl Pidl {
    fn parse(path: &Path) -> Result<Self> {
        let mut pidl = std::ptr::null_mut();
        unsafe { SHParseDisplayName(&HSTRING::from(path.as_os_str()), None, &mut pidl, 0, None)? };
        Ok(Self(pidl))
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0 as *const c_void)) };
    }
}

/// 同じフォルダの項目に対するシェルの部品を取る。
fn ui_object<T: Interface>(hwnd: HWND, folder: &Path, names: &[OsString]) -> Result<T> {
    let items = names.iter().map(|name| Pidl::parse(&folder.join(name))).collect::<Result<Vec<_>>>()?;
    let first = items.first().ok_or(windows::core::Error::empty())?;
    unsafe {
        let parent: IShellFolder = SHBindToParent(first.0, None)?;
        let children: Vec<*const ITEMIDLIST> = items.iter().map(|item| ILFindLastID(item.0).cast_const()).collect();
        parent.GetUIObjectOf(hwnd, &children, None)
    }
}

/// サブメニューの描画に必要なメッセージを、メニューの部品へ渡す。
unsafe extern "system" fn forward_menu_messages(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    menu: usize,
) -> LRESULT {
    unsafe {
        if matches!(message, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
            let menu = &*(menu as *const IContextMenu);
            if let Ok(menu3) = menu.cast::<IContextMenu3>() {
                let mut result = LRESULT(0);
                if menu3.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)).is_ok() {
                    return result;
                }
            } else if let Ok(menu2) = menu.cast::<IContextMenu2>()
                && menu2.HandleMenuMsg(message, wparam, lparam).is_ok()
            {
                return LRESULT(0);
            }
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

/// 右クリックメニューで選ばれた結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuChoice {
    None,
    Invoked,
    /// 「名前の変更」はエクスプローラーの画面の中でしか動かないので、アプリ側で行う
    Rename,
}

/// メニューの項目の内部名を読む。
fn command_verb(menu: &IContextMenu, index: usize) -> Option<String> {
    let mut buffer = [0u8; 64];
    let result = unsafe { menu.GetCommandString(index, GCS_VERBA, None, PSTR(buffer.as_mut_ptr()), buffer.len() as u32) };
    result.ok()?;
    let length = buffer.iter().position(|&c| c == 0)?;
    Some(String::from_utf8_lossy(&buffer[..length]).into_owned())
}

fn track(hwnd: HWND, menu: &IContextMenu) -> Result<MenuChoice> {
    unsafe {
        let popup = CreatePopupMenu()?;
        menu.QueryContextMenu(popup, 0, FIRST_COMMAND, LAST_COMMAND, CMF_NORMAL | CMF_EXPLORE).ok()?;
        let mut point = POINT::default();
        GetCursorPos(&mut point)?;
        let _ = SetForegroundWindow(hwnd);
        let data = menu as *const IContextMenu as usize;
        let _ = SetWindowSubclass(hwnd, Some(forward_menu_messages), 1, data);
        let flags = (TPM_RETURNCMD | TPM_RIGHTBUTTON).0;
        let command = TrackPopupMenuEx(popup, flags, point.x, point.y, hwnd, None).0 as u32;
        let _ = RemoveWindowSubclass(hwnd, Some(forward_menu_messages), 1);
        let verb = (command >= FIRST_COMMAND).then(|| command_verb(menu, (command - FIRST_COMMAND) as usize)).flatten();
        let invoked = if verb.as_deref() == Some("rename") {
            Ok(MenuChoice::Rename)
        } else if command >= FIRST_COMMAND {
            let info = CMINVOKECOMMANDINFO {
                cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                hwnd,
                // 上位が0のポインタは、文字列ではなく番号として扱われる
                lpVerb: PCSTR((command - FIRST_COMMAND) as usize as *const u8),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            menu.InvokeCommand(&info).map(|()| MenuChoice::Invoked)
        } else {
            Ok(MenuChoice::None)
        };
        let _ = DestroyMenu(popup);
        invoked
    }
}

/// メニューを画面に出さずに組み立てる。シェル拡張を読み込ませ、最初の右クリックを速くする。
pub fn preload_items_menu(folder: &Path, names: &[OsString]) -> Result<()> {
    let menu: IContextMenu = ui_object(HWND::default(), folder, names)?;
    unsafe {
        let popup = CreatePopupMenu()?;
        let result = menu.QueryContextMenu(popup, 0, FIRST_COMMAND, LAST_COMMAND, CMF_NORMAL | CMF_EXPLORE).ok();
        let _ = DestroyMenu(popup);
        result
    }
}

/// 項目の右クリックメニューを出し、選ばれた項目を実行する。
pub fn show_items_menu(hwnd: isize, folder: &Path, names: &[OsString]) -> Result<MenuChoice> {
    let hwnd = to_hwnd(hwnd);
    track(hwnd, &ui_object::<IContextMenu>(hwnd, folder, names)?)
}

/// フォルダの余白の右クリックメニュー（新規作成、貼り付けなど）を出す。
pub fn show_folder_menu(hwnd: isize, folder: &Path) -> Result<MenuChoice> {
    let hwnd = to_hwnd(hwnd);
    let pidl = Pidl::parse(folder)?;
    let menu: IContextMenu = unsafe {
        let desktop = SHGetDesktopFolder()?;
        let shell_folder: IShellFolder = desktop.BindToObject(pidl.0, None)?;
        shell_folder.CreateViewObject(hwnd)?
    };
    track(hwnd, &menu)
}

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if key_state.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// ドラッグ中にカーソルへ付ける札を作る。絵の所有権はシェルへ渡す。
fn attach_drag_label(data: &IDataObject, label: &str) -> Result<()> {
    const HEIGHT: i32 = 30;
    const ALPHA: u32 = 0xE0;
    let text: Vec<u16> = label.encode_utf16().collect();
    let width = (text.len() as i32 * 14 + 24).clamp(80, 340);
    unsafe {
        let helper: IDragSourceHelper = CoCreateInstance(&CLSID_DRAG_DROP_HELPER, None, CLSCTX_INPROC_SERVER)?;
        let dc = CreateCompatibleDC(None);
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -HEIGHT,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (width * HEIGHT) as usize);
        let [red, green, blue] = [0x0au32, 0x64, 0xd8].map(|c| c * ALPHA / 255);
        pixels.fill((ALPHA << 24) | (red << 16) | (green << 8) | blue);
        let font = CreateFontW(-14, 0, 0, 0, 500, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY, 0, w!("Yu Gothic UI"));
        let previous_font = SelectObject(dc, HGDIOBJ(font.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00ff_ffff));
        let _ = TextOutW(dc, 10, 6, &text);
        // GDIの文字は透明度を0にするので、書き戻す
        for pixel in pixels.iter_mut() {
            *pixel |= ALPHA << 24;
        }
        SelectObject(dc, previous_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        SelectObject(dc, previous);
        let _ = DeleteDC(dc);
        let image = SHDRAGIMAGE {
            sizeDragImage: SIZE { cx: width, cy: HEIGHT },
            ptOffset: POINT { x: 0, y: 0 },
            hbmpDragImage: bitmap,
            crColorKey: COLORREF(0xffff_ffff),
        };
        let result = helper.InitializeFromBitmap(&image, data);
        if result.is_err() {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
        }
        result
    }
}

/// 項目をOLEのドラッグとして持ち出す。離すまで戻らない。`label` はカーソルに付ける文字。
pub fn drag_items(hwnd: isize, folder: &Path, names: &[OsString], label: &str) -> Result<()> {
    let data: IDataObject = ui_object(to_hwnd(hwnd), folder, names)?;
    // 札は飾りなので、付けられなくてもドラッグは続ける
    let _ = attach_drag_label(&data, label);
    let source: IDropSource = DropSource.into();
    let mut effect = DROPEFFECT::default();
    unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect).ok() }
}
