//! アプリアイコンを SVG から作る道具。
//!
//! 使い方:
//!   cargo run -p icongen -- ico <小さい用.svg> <大きい用.svg> <出力.ico>
//!   cargo run -p icongen -- sheet <出力.png> <a.svg> [<b.svg> ...]
//!
//! `ico` は 16, 20, 24, 32 px を小さい用の SVG から、40, 48, 64, 256 px を大きい用の SVG から描く。
//! 小さいサイズは線を太く描き直した SVG を使い、縮小で潰れないようにする。
//! `sheet` は案を見比べるための画像で、各 SVG を 16, 32, 256 px で描いて横に並べる。

use std::path::Path;

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::{ExtendedColorType, Rgba, RgbaImage, imageops};
use resvg::{tiny_skia, usvg};

const SMALL_SIZES: [u32; 4] = [16, 20, 24, 32];
const LARGE_SIZES: [u32; 4] = [40, 48, 64, 256];

fn render(svg: &Path, size: u32) -> RgbaImage {
    let data = std::fs::read(svg).unwrap_or_else(|e| panic!("{} を読めない: {e}", svg.display()));
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .unwrap_or_else(|e| panic!("{} を解釈できない: {e}", svg.display()));
    let mut pixmap = tiny_skia::Pixmap::new(size, size).unwrap();
    let scale = size as f32 / tree.size().width().max(tree.size().height());
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia は乗算済みアルファで返すので、ICO と PNG のために戻す
    let mut image = RgbaImage::new(size, size);
    for (pixel, color) in image.pixels_mut().zip(pixmap.pixels()) {
        let c = color.demultiply();
        *pixel = Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    image
}

fn write_ico(small: &Path, large: &Path, out: &Path) {
    let frames: Vec<IcoFrame> = SMALL_SIZES
        .iter()
        .map(|&s| (small, s))
        .chain(LARGE_SIZES.iter().map(|&s| (large, s)))
        .map(|(svg, size)| {
            let image = render(svg, size);
            IcoFrame::as_png(image.as_raw(), size, size, ExtendedColorType::Rgba8).unwrap()
        })
        .collect();
    let file = std::fs::File::create(out).unwrap();
    IcoEncoder::new(file).encode_images(&frames).unwrap();
}

fn write_sheet(out: &Path, svgs: &[String]) {
    // 各案を 1 行に: 16 px と 32 px は原寸と 4 倍拡大、256 px は原寸
    let row_height = 256 + 16;
    let width = 16 + 64 + 16 + 128 + 16 + 256 + 16 + 16 + 32 + 16;
    let mut sheet = RgbaImage::from_pixel(
        width,
        row_height * svgs.len() as u32,
        Rgba([246, 246, 247, 255]),
    );
    for (i, svg) in svgs.iter().enumerate() {
        let svg = Path::new(svg);
        let top = (i as u32 * row_height + 8) as i64;
        let s16 = render(svg, 16);
        let s32 = render(svg, 32);
        let big = render(svg, 256);
        let mut x = 16i64;
        let up16 = imageops::resize(&s16, 64, 64, imageops::FilterType::Nearest);
        imageops::overlay(&mut sheet, &up16, x, top + 96);
        x += 64 + 16;
        let up32 = imageops::resize(&s32, 128, 128, imageops::FilterType::Nearest);
        imageops::overlay(&mut sheet, &up32, x, top + 64);
        x += 128 + 16;
        imageops::overlay(&mut sheet, &big, x, top);
        x += 256 + 16;
        imageops::overlay(&mut sheet, &s16, x, top + 120);
        imageops::overlay(&mut sheet, &s32, x + 16 + 8, top + 112);
    }
    sheet.save(out).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ico") if args.len() == 4 => write_ico(
            Path::new(&args[1]),
            Path::new(&args[2]),
            Path::new(&args[3]),
        ),
        Some("sheet") if args.len() >= 3 => write_sheet(Path::new(&args[1]), &args[2..]),
        _ => {
            eprintln!(
                "使い方: icongen ico <small.svg> <large.svg> <out.ico> | icongen sheet <out.png> <svg>..."
            );
            std::process::exit(2);
        }
    }
}
