use filer_shell::{Operation, Prompt, init_thread, inverse, run_operation, run_recording_undo, unique_folder_name, valid_file_name};
use std::path::PathBuf;

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
    init_thread();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("素材.txt");
    std::fs::write(&file, "x").unwrap();
    let target = dir.path().join("先");
    std::fs::create_dir(&target).unwrap();
    (dir, file, target)
}

#[test]
fn copies_into_folder() {
    let (_dir, file, target) = setup();
    let op = Operation::Copy { items: vec![file.clone()], destination: target.clone() };
    run_operation(&op, Prompt::Silent).unwrap();
    assert!(file.exists());
    assert!(target.join("素材.txt").exists());
}

#[test]
fn moves_into_folder() {
    let (_dir, file, target) = setup();
    let op = Operation::Move { items: vec![file.clone()], destination: target.clone() };
    run_operation(&op, Prompt::Silent).unwrap();
    assert!(!file.exists());
    assert!(target.join("素材.txt").exists());
}

#[test]
fn renames_a_file() {
    let (dir, file, _target) = setup();
    let op = Operation::Rename { item: file.clone(), name: "新しい.txt".into() };
    run_operation(&op, Prompt::Silent).unwrap();
    assert!(!file.exists());
    assert!(dir.path().join("新しい.txt").exists());
}

#[test]
fn creates_a_folder() {
    let (dir, _file, _target) = setup();
    let op = Operation::NewFolder { parent: dir.path().into(), name: "新しいフォルダー".into() };
    run_operation(&op, Prompt::Silent).unwrap();
    assert!(dir.path().join("新しいフォルダー").is_dir());
}

#[test]
fn moves_file_to_recycle_bin() {
    let (_dir, file, _target) = setup();
    run_operation(&Operation::Recycle { items: vec![file.clone()] }, Prompt::Silent).unwrap();
    assert!(!file.exists());
}

#[test]
fn missing_source_reports_error() {
    let (dir, _file, target) = setup();
    let op = Operation::Copy { items: vec![dir.path().join("none")], destination: target };
    assert!(run_operation(&op, Prompt::Silent).is_err());
}

#[test]
fn unique_name_skips_existing_folders() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(unique_folder_name(dir.path()), "新しいフォルダー");
    std::fs::create_dir(dir.path().join("新しいフォルダー")).unwrap();
    assert_eq!(unique_folder_name(dir.path()), "新しいフォルダー (2)");
}

#[test]
fn copy_in_same_folder_keeps_both() {
    let (dir, file, _target) = setup();
    let op = Operation::Copy { items: vec![file.clone()], destination: dir.path().into() };
    run_operation(&op, Prompt::Silent).unwrap();
    let files = std::fs::read_dir(dir.path()).unwrap().filter(|e| e.as_ref().unwrap().path().is_file()).count();
    assert_eq!(files, 2);
}

/// 実行するとクリップボードの中身を置き換えるので、手で指定した時だけ動かす。
#[test]
#[ignore]
fn clipboard_keeps_paths_and_cut_flag() {
    let paths = vec![PathBuf::from(r"C:\素材\a.txt"), PathBuf::from(r"C:\b.txt")];
    filer_shell::set_clipboard_files(&paths, true).unwrap();
    assert_eq!(filer_shell::clipboard_files(), Some((paths, true)));
}

#[test]
fn rename_is_undone_by_renaming_back() {
    let op = Operation::Rename { item: r"C:\a\old.txt".into(), name: "new.txt".into() };
    assert_eq!(inverse(&op), Some(Operation::Rename { item: r"C:\a\new.txt".into(), name: "old.txt".into() }));
}

#[test]
fn move_is_undone_by_moving_back() {
    let op = Operation::Move { items: vec![r"C:\a\x".into()], destination: r"C:\b".into() };
    let back = Operation::Move { items: vec![r"C:\b\x".into()], destination: r"C:\a".into() };
    assert_eq!(inverse(&op), Some(back));
}

#[test]
fn copy_is_undone_by_recycling_the_copies() {
    let op = Operation::Copy { items: vec![r"C:\a\x".into()], destination: r"C:\b".into() };
    assert_eq!(inverse(&op), Some(Operation::Recycle { items: vec![r"C:\b\x".into()] }));
}

#[test]
fn recycle_and_same_folder_copy_cannot_be_undone() {
    assert_eq!(inverse(&Operation::Recycle { items: vec![r"C:\a".into()] }), None);
    let op = Operation::Copy { items: vec![r"C:\a\x".into()], destination: r"C:\a".into() };
    assert_eq!(inverse(&op), None);
}

#[test]
fn undo_is_offered_when_nothing_collided() {
    let (_dir, file, target) = setup();
    let op = Operation::Copy { items: vec![file], destination: target.clone() };
    let undo = run_recording_undo(&op, Prompt::Silent).unwrap();
    assert_eq!(undo, Some(Operation::Recycle { items: vec![target.join("素材.txt")] }));
}

#[test]
fn collision_gives_no_undo() {
    let (_dir, file, target) = setup();
    std::fs::write(target.join("素材.txt"), "existing").unwrap();
    let op = Operation::Copy { items: vec![file], destination: target };
    assert_eq!(run_recording_undo(&op, Prompt::Silent).unwrap(), None);
}

#[test]
fn rename_collision_gives_no_undo() {
    let (dir, file, _target) = setup();
    std::fs::write(dir.path().join("既存.txt"), "existing").unwrap();
    let op = Operation::Rename { item: file, name: "既存.txt".into() };
    assert_eq!(run_recording_undo(&op, Prompt::Silent).ok().flatten(), None);
}

#[test]
fn same_folder_copy_ignores_case() {
    let op = Operation::Copy { items: vec![r"C:\A\x".into()], destination: r"c:\a".into() };
    assert_eq!(inverse(&op), None);
}

#[test]
fn watcher_reports_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let _job = filer_shell::watch_folder(dir.path().into(), move || {
        let _ = tx.send(());
    });
    std::thread::sleep(std::time::Duration::from_millis(200));
    std::fs::write(dir.path().join("a"), "x").unwrap();
    assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_ok());
}

#[test]
fn file_names_are_validated() {
    assert!(valid_file_name("素材 01.mp4"));
    assert!(!valid_file_name(""));
    assert!(!valid_file_name("   "));
    assert!(!valid_file_name("a/b"));
    assert!(!valid_file_name("a:b"));
    assert!(!valid_file_name("."));
    assert!(!valid_file_name("CON"));
}
