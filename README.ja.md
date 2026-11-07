# Binder for Windows

[English](README.md)

Binder は、Windows 10/11 で動く Finder 風のファイラーです。
Rust と [GPUI](https://github.com/zed-industries/zed) で書いています。

- リスト、カラム、アイコンの3つの表示、タブ（Ctrl+T）、よく使う項目とドライブのサイドバー、パス欄
- Space キーのクイックルック（画像、PDF、動画と音声、テキスト、Zip とフォルダのツリー）
- コピー、移動、名前の変更、ごみ箱への削除、取り消し、ドラッグ、Windows の右クリックメニュー
- 今のフォルダの中の検索（Ctrl+F）
- 希望すれば、フォルダと Win+E をエクスプローラーの代わりに Binder で開く

Binder はオープンソースではなく、**source-available** のソフトウェアです。
[PolyForm Noncommercial License 1.0.0](LICENSE) のもとで、商用でない目的に限り、使う、変える、配ることができます。
Apache License 2.0 のまま残る部分は [NOTICE](NOTICE) に書いてあります。

## インストール

[Releases](https://github.com/3208noname-arch/binder-for-windows/releases) から、次のどちらかを取ります。

- `Binder-Setup-<版>.exe`：今のユーザーだけに、`%LOCALAPPDATA%\Programs\Binder` へ入れます。管理者の権限は要りません。
- `Binder-<版>-x64.zip`：好きな場所に展開し、`binder.exe` を起動します。

### SmartScreen の警告

ファイルにはコード署名をしていません。
そのため、起動すると「Windows によって PC が保護されました」と出ることがあります。
下の手順でファイルを確かめてから、「詳細情報」を押し、「実行」を押してください。

### ファイルの確かめ方

各リリースのリリースノートと `SHA256SUMS.txt` に、ファイルごとの SHA-256 を載せています。
PowerShell で次のように出した値と比べてください。

```powershell
Get-FileHash .\Binder-Setup-0.1.0.exe -Algorithm SHA256
```

## エクスプローラーの代わりに Binder で開く

設定（歯車のボタン）の「既定のファイラー」で、スイッチをオンにします。
オンにすると、他のアプリがフォルダを「開く」操作で、Binder が開きます（PowerShell の `Start-Process C:\Windows` など）。

Win+E にも、他のエクスプローラーの代わりのアプリと同じ方法で登録します。
ただし、PC によっては、Win+E と、デスクトップやエクスプローラーの中でのフォルダのダブルクリックが、エクスプローラーのまま開きます。
QTTabBar を入れた PC で、そうなることを確かめています。

仕組みは次のとおりです。

- 書き込むのは `HKEY_CURRENT_USER` の中だけです（`Software\Classes\Folder\shell\open\command`、`Software\Classes\Folder\shell\explore\command`、`Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command`）。
- 他のアプリ（QTTabBar など）が `Software\Classes\Folder\shell` の既定の動作を変えている時は、`open` に戻します。そのままでは「開く」が呼ばれないためです。
- 書き込む前に、`Folder\shell` のキー全体と CLSID のキーを `%LOCALAPPDATA%\binder\explorer-backup.reg` に書き出します。スイッチをオフにすると、このファイルを取り込み、キーを書き出した時の内容へそのまま戻します。ファイルをダブルクリックしても、手で戻せます。
- ごみ箱やコントロールパネルのような、実在のフォルダでない場所は、エクスプローラーに渡します。
- Binder をアンインストールすると、スイッチもオフに戻ります。

## 設定の保存先

設定は `%LOCALAPPDATA%\binder\settings.sqlite` に保存します。
試作版の `%LOCALAPPDATA%\filer` に設定があれば、最初の起動の時にそこから写します。

## ソースからのビルド

必要なものは次のとおりです。

- Windows 10/11 の x64
- Rust（stable、edition 2024）
- Windows SDK（`fxc.exe` を使います）。ビルドは `Windows Kits\10\bin\<版>\x64\fxc.exe` のうち、最も新しい版を探します。別の `fxc.exe` を使う時は、環境変数 `GPUI_FXC_PATH` に指定します。

```powershell
cargo build --release -p filer-ui
cargo test --release --workspace --exclude gpui
```

できたプログラムは `target\release\binder.exe` です。
crate の名前は、内部の名前の `filer-*` のままにしています。

`vendor/gpui` は、手を加えた GPUI 0.2.2 です。`[patch.crates-io]` で使っています。

## ライセンス

- Binder：[PolyForm Noncommercial License 1.0.0](LICENSE)
- `vendor/gpui` と `crates/ui/src/text_input.rs`：Apache License 2.0（[NOTICE](NOTICE) を参照）

「Finder」は Apple Inc. の商標です。
Binder は Apple とは関係ありません。
