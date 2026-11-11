//! exe にアプリアイコンを埋め込む。アイコンは assets/icon/README.md の手順で SVG から作る。

fn main() {
    println!("cargo:rerun-if-changed=binder.rc");
    println!("cargo:rerun-if-changed=../../assets/icon/binder.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("binder.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
