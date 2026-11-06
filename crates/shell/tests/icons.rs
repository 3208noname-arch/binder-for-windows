use filer_shell::{drives, favorites, init_thread, load_icon};
use std::path::Path;

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G'];

#[test]
fn loads_icon_of_text_file_as_png() {
    init_thread();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("素材.txt");
    std::fs::write(&file, "x").unwrap();
    let png = load_icon(&file, 32, false).expect("icon");
    assert!(png.starts_with(PNG_MAGIC));
}

#[test]
fn loads_icon_of_folder() {
    init_thread();
    assert!(load_icon(Path::new(r"C:\Windows"), 32, false).is_some());
}

#[test]
fn missing_file_has_no_icon() {
    init_thread();
    assert!(load_icon(Path::new(r"C:\no\such\file.txt"), 32, false).is_none());
}

#[test]
fn drives_include_system_drive() {
    assert!(drives().iter().any(|d| d == Path::new(r"C:\")));
}

#[test]
fn favorites_are_existing_folders() {
    let list = favorites();
    assert!(!list.is_empty());
    assert!(list.iter().all(|(name, path)| !name.is_empty() && path.is_dir()));
}
