# アプリアイコン

`binder.ico` は次の 2 つの SVG から作る。SVG が正本で、`.ico` は生成物だが、ビルドに要るのでリポジトリに入れる。

- `binder.svg`: 40, 48, 64, 256 px 用。
- `binder-small.svg`: 16, 20, 24, 32 px 用。16 px の画素に合わせて描き直した版で、縮小してもリングが潰れない。

作り直す時は、リポジトリの根で次を実行する。

```
cargo run -p icongen -- ico assets/icon/binder-small.svg assets/icon/binder.svg assets/icon/binder.ico
```

見比べ用の画像（16 px と 32 px の 4 倍拡大、256 px、原寸）は次で作れる。

```
cargo run -p icongen -- sheet sheet.png assets/icon/binder-small.svg assets/icon/binder.svg
```

`.ico` は `crates/ui/build.rs` が `binder.exe` に ID 1 で埋め込み、gpui がそれをウィンドウとタスクバーに使う。インストーラーは `installer/binder.iss` の `SetupIconFile` で同じ `.ico` を使う。
