//! 速度の実測用に、空ファイルが並ぶフォルダを作る。
//! 使い方: bench_list <フォルダ> <件数>

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("folder");
    let count: usize = args.next().expect("count").parse().expect("number");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..count {
        std::fs::write(format!("{dir}\\素材_{i:06}_映像データ.mp4"), "").unwrap();
    }
}
