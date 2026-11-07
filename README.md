# Binder for Windows

[日本語](README.ja.md)

Binder is a fast, Finder-like file manager for Windows 10/11, written in Rust with [GPUI](https://github.com/zed-industries/zed).

- List, column and icon views, tabs (Ctrl+T), a sidebar of favorites and drives, and a path bar
- Quick Look with the Space key: images, PDF, video and audio, text, and the tree of a Zip file or a folder
- Copy, move, rename, delete to the Recycle Bin, undo, drag and drop, and the Windows context menu
- Search in the current folder (Ctrl+F)
- Optional: open folders and Win+E in Binder instead of File Explorer

Binder is **source-available**, not open source. You may use, change and share it for noncommercial purposes under the [PolyForm Noncommercial License 1.0.0](LICENSE). See [NOTICE](NOTICE) for the parts that stay under the Apache License 2.0.

## Install

Download one of these from [Releases](https://github.com/3208noname-arch/binder-for-windows/releases):

- `Binder-Setup-<version>.exe`: installs for the current user only into `%LOCALAPPDATA%\Programs\Binder`. No administrator rights are needed.
- `Binder-<version>-x64.zip`: unzip anywhere and run `binder.exe`.

### SmartScreen warning

The files are not code-signed, so Windows SmartScreen may show "Windows protected your PC". Check the file first (see below), then click **More info** and **Run anyway**.

### Check the download

Each release lists the SHA-256 of its files in the release notes and in `SHA256SUMS.txt`. Compare them in PowerShell:

```powershell
Get-FileHash .\Binder-Setup-0.1.0.exe -Algorithm SHA256
```

## Open folders in Binder instead of File Explorer

Open Settings (the gear button) → **Default file manager** and turn the switch on. Binder then opens folders that other apps open with the "open" action, such as `Start-Process C:\Windows` in PowerShell.

Binder also registers itself for Win+E, as other Explorer replacements do. On some PCs, Win+E and double-clicking a folder on the desktop or inside File Explorer still open File Explorer. This was seen on a PC with QTTabBar installed.

How it works:

- Binder writes only to `HKEY_CURRENT_USER`: `Software\Classes\Folder\shell\open\command`, `Software\Classes\Folder\shell\explore\command`, and `Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command`.
- If another app (such as QTTabBar) has changed the default action of `Software\Classes\Folder\shell`, Binder sets it back to `open`. Otherwise "open" would not be called.
- Before writing, Binder exports the whole `Folder\shell` key and the CLSID key to `%LOCALAPPDATA%\binder\explorer-backup.reg`. Turning the switch off imports this file, so the keys return to exactly what they were. You can also double-click the file to restore them by hand.
- Places that are not real folders, such as the Recycle Bin and Control Panel, are passed to File Explorer.
- Uninstalling Binder turns the switch off.

## Settings

Settings are saved in `%LOCALAPPDATA%\binder\settings.sqlite`. Settings from earlier test builds in `%LOCALAPPDATA%\filer` are copied there on the first start.

## Build from source

Requirements:

- Windows 10/11 x64
- Rust (stable, edition 2024)
- Windows SDK (for `fxc.exe`). The build finds the newest `Windows Kits\10\bin\<version>\x64\fxc.exe`. Set `GPUI_FXC_PATH` to use another one.

```powershell
cargo build --release -p filer-ui
cargo test --release --workspace --exclude gpui
```

The program is `target\release\binder.exe`. The crates keep the internal names `filer-*`.

`vendor/gpui` is a modified copy of GPUI 0.2.2 used through `[patch.crates-io]`.

## License

- Binder: [PolyForm Noncommercial License 1.0.0](LICENSE)
- `vendor/gpui` and `crates/ui/src/text_input.rs`: Apache License 2.0 (see [NOTICE](NOTICE))

"Finder" is a trademark of Apple Inc. Binder is not affiliated with Apple.
