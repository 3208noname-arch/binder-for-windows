use std::ffi::{OsString, c_void};
use std::path::Path;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DoDragDrop, IDropSource, IDropSource_Impl};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_EXPLORE, CMF_NORMAL, CMINVOKECOMMANDINFO, DefSubclassProc, IContextMenu, IContextMenu2, IContextMenu3,
    ILFindLastID, IShellFolder, RemoveWindowSubclass, SHBindToParent, SHGetDesktopFolder, SHParseDisplayName,
    SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, SW_SHOWNORMAL, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};
use windows::core::{BOOL, HRESULT, HSTRING, Interface, PCSTR, Result, implement};
use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};

const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7fff;

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

fn track(hwnd: HWND, menu: &IContextMenu) -> Result<bool> {
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
        let invoked = if command >= FIRST_COMMAND {
            let info = CMINVOKECOMMANDINFO {
                cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                hwnd,
                // 上位が0のポインタは、文字列ではなく番号として扱われる
                lpVerb: PCSTR((command - FIRST_COMMAND) as usize as *const u8),
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            menu.InvokeCommand(&info).map(|()| true)
        } else {
            Ok(false)
        };
        let _ = DestroyMenu(popup);
        invoked
    }
}

/// 項目の右クリックメニューを出し、選ばれた項目を実行する。実行したら真を返す。
pub fn show_items_menu(hwnd: isize, folder: &Path, names: &[OsString]) -> Result<bool> {
    let hwnd = to_hwnd(hwnd);
    track(hwnd, &ui_object::<IContextMenu>(hwnd, folder, names)?)
}

/// フォルダの余白の右クリックメニュー（新規作成、貼り付けなど）を出す。
pub fn show_folder_menu(hwnd: isize, folder: &Path) -> Result<bool> {
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

/// 項目をOLEのドラッグとして持ち出す。離すまで戻らない。
pub fn drag_items(hwnd: isize, folder: &Path, names: &[OsString]) -> Result<()> {
    let data: IDataObject = ui_object(to_hwnd(hwnd), folder, names)?;
    let source: IDropSource = DropSource.into();
    let mut effect = DROPEFFECT::default();
    unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect).ok() }
}
