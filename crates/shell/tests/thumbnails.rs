use filer_shell::{Made, Thumb, ThumbJob, ThumbKind, ThumbPool, ThumbStore, init_thread, make_thumbnail};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

fn write_png(path: &Path, width: u32, height: u32) {
    let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&vec![200u8; (width * height * 4) as usize]).unwrap();
}

fn never() -> bool {
    false
}

fn job(path: &Path, modified: i64, kind: ThumbKind) -> ThumbJob {
    ThumbJob { key: "k".into(), path: path.to_path_buf(), side: 128, kind, modified, length: 1, keep: false }
}

fn pool(cache: PathBuf) -> (ThumbPool, Receiver<(String, Thumb)>) {
    let (sender, results) = channel();
    let sender = std::sync::Mutex::new(sender);
    (ThumbPool::start(Some(ThumbStore::new(cache)), move |key, thumb| drop(sender.lock().unwrap().send((key, thumb)))), results)
}

fn next(results: &Receiver<(String, Thumb)>) -> Thumb {
    results.recv_timeout(Duration::from_secs(30)).expect("no result").1
}

#[test]
fn a_png_shrinks_to_the_requested_side_and_keeps_its_shape() {
    init_thread();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("large.png");
    write_png(&file, 1000, 500);
    let Made::Picture(bytes) = make_thumbnail(&file, 128, ThumbKind::Image, &never) else { panic!("no picture") };
    let reader = png::Decoder::new(bytes.as_slice()).read_info().unwrap();
    assert_eq!((reader.info().width, reader.info().height), (128, 64));
}

#[test]
fn broken_and_empty_files_are_refused_without_a_crash() {
    init_thread();
    let dir = tempfile::tempdir().unwrap();
    for (name, kind, bytes) in [
        ("broken.jpg", ThumbKind::Image, b"not a jpeg".as_slice()),
        ("broken.mp4", ThumbKind::Video, b"not a video at all".as_slice()),
        ("broken.mkv", ThumbKind::Video, &[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0]),
        ("empty.png", ThumbKind::Image, b"".as_slice()),
        ("empty.mp4", ThumbKind::Video, b"".as_slice()),
    ] {
        let file = dir.path().join(name);
        std::fs::write(&file, bytes).unwrap();
        let made = make_thumbnail(&file, 128, kind, &never);
        assert!(matches!(made, Made::Unsupported), "{name}: {made:?}");
    }
}

#[test]
fn a_missing_file_is_reported_as_unreachable() {
    init_thread();
    let made = make_thumbnail(Path::new(r"C:\no\such\file.mp4"), 128, ThumbKind::Video, &never);
    assert!(matches!(made, Made::Unreachable), "{made:?}");
}

#[test]
fn a_cancelled_request_does_no_work() {
    let made = make_thumbnail(Path::new(r"C:\no\such\file.mp4"), 128, ThumbKind::Video, &|| true);
    assert!(matches!(made, Made::Cancelled), "{made:?}");
}

#[test]
fn the_second_pool_answers_from_disk_and_a_changed_file_is_made_again() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let file = dir.path().join("picture.png");
    write_png(&file, 400, 300);
    {
        let (pool, results) = pool(cache.clone());
        pool.request(job(&file, 1, ThumbKind::Image));
        assert!(matches!(next(&results), Thumb::Picture(_)));
    }
    // 元を消しても、保存済みなら絵が出る
    std::fs::remove_file(&file).unwrap();
    let (pool, results) = pool(cache);
    pool.request(job(&file, 1, ThumbKind::Image));
    assert!(matches!(next(&results), Thumb::Picture(_)));
    // 更新日時が変われば保存済みを使わず作りに行き、元が無いので届かないと分かる
    pool.request(job(&file, 2, ThumbKind::Image));
    assert!(matches!(next(&results), Thumb::Later));
}

#[test]
fn a_failure_is_remembered_across_pools() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let file = dir.path().join("broken.png");
    std::fs::write(&file, b"junk").unwrap();
    {
        let (pool, results) = pool(cache.clone());
        pool.request(job(&file, 1, ThumbKind::Image));
        assert!(matches!(next(&results), Thumb::Unavailable));
    }
    // 中身が正しくなっても、更新日時が同じなら作り直さない
    write_png(&file, 10, 10);
    let (pool, results) = pool(cache);
    pool.request(job(&file, 1, ThumbKind::Image));
    assert!(matches!(next(&results), Thumb::Unavailable));
}
