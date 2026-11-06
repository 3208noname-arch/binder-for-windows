use filer_shell::{ListEvent, compare_names, long_path, start_list};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel, sync_channel};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);

fn listen(path: PathBuf) -> (filer_shell::ListJob, Receiver<ListEvent>) {
    let (tx, rx) = channel();
    let job = start_list(path, move |event| {
        let _ = tx.send(event);
    });
    (job, rx)
}

/// 完了までの名前を集める。失敗の時はそのメッセージを返す。
fn collect(rx: Receiver<ListEvent>) -> (Vec<String>, usize, Option<String>) {
    let (mut names, mut batches) = (Vec::new(), 0);
    loop {
        match rx.recv_timeout(WAIT).expect("event") {
            ListEvent::Batch(b) => {
                batches += 1;
                names.extend(b.into_iter().map(|e| e.name));
            }
            ListEvent::Done => return (names, batches, None),
            ListEvent::Failed(m) => return (names, batches, Some(m)),
        }
    }
}

fn touch(dir: &Path, count: usize) {
    for i in 0..count {
        std::fs::write(dir.join(format!("f{i}.txt")), "").unwrap();
    }
}

#[test]
fn lists_all_entries_in_batches() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), 5000);
    let (_job, rx) = listen(dir.path().into());
    let (names, batches, error) = collect(rx);
    assert_eq!(error, None);
    assert_eq!(names.len(), 5000);
    assert!(batches >= 2);
}

#[test]
fn reports_size_and_folder_flag() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "12345").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let (_job, rx) = listen(dir.path().into());
    let mut entries = Vec::new();
    loop {
        match rx.recv_timeout(WAIT).unwrap() {
            ListEvent::Batch(b) => entries.extend(b),
            ListEvent::Done => break,
            ListEvent::Failed(m) => panic!("{m}"),
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(entries[0].size, 5);
    assert!(!entries[0].is_dir);
    assert!(entries[0].modified > 0);
    assert!(entries[1].is_dir);
}

#[test]
fn skips_dot_entries() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), 1);
    let (_job, rx) = listen(dir.path().into());
    let (names, _, _) = collect(rx);
    assert_eq!(names, ["f0.txt"]);
}

#[test]
fn empty_folder_is_done_without_entries() {
    let dir = tempfile::tempdir().unwrap();
    let (_job, rx) = listen(dir.path().into());
    let (names, _, error) = collect(rx);
    assert!(names.is_empty());
    assert_eq!(error, None);
}

#[test]
fn missing_path_reports_failed() {
    let dir = tempfile::tempdir().unwrap();
    let (_job, rx) = listen(dir.path().join("none"));
    let (_, _, error) = collect(rx);
    assert!(error.is_some());
}

#[test]
fn dropping_job_stops_events() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), 5000);
    let (tx, rx) = channel();
    let (gate_tx, gate_rx) = sync_channel::<()>(0);
    let job = start_list(dir.path().into(), move |event| {
        let _ = tx.send(event);
        let _ = gate_rx.recv();
    });
    assert!(matches!(rx.recv_timeout(WAIT).unwrap(), ListEvent::Batch(_)));
    drop(job);
    drop(gate_tx);
    assert!(rx.recv_timeout(Duration::from_secs(2)).is_err());
}

#[test]
fn lists_path_over_260_chars() {
    let dir = tempfile::tempdir().unwrap();
    let mut path = dir.path().to_path_buf();
    while path.as_os_str().len() < 300 {
        path.push("x".repeat(40));
    }
    std::fs::create_dir_all(long_path(&path)).unwrap();
    std::fs::write(long_path(&path.join("a.txt")), "").unwrap();
    let (_job, rx) = listen(path);
    let (names, _, error) = collect(rx);
    assert_eq!(error, None);
    assert_eq!(names, ["a.txt"]);
}

#[test]
fn long_path_keeps_short_paths_and_handles_unc() {
    assert_eq!(long_path(Path::new(r"C:\a")), Path::new(r"C:\a"));
    let share = format!(r"\\nas\share\{}", "x".repeat(300));
    assert_eq!(long_path(Path::new(&share)), PathBuf::from(format!(r"\\?\UNC\nas\share\{}", "x".repeat(300))));
}

#[test]
fn format_time_is_local_year_to_minute() {
    // 2026-07-01 00:00 UTC。どのタイムゾーンでも2026年の6月か7月になる
    let text = filer_shell::format_time(134_273_376_000_000_000);
    assert_eq!(text.len(), 16, "{text}");
    assert!(text.starts_with("2026/0"), "{text}");
    assert_eq!(filer_shell::format_time(0), "");
}

#[test]
fn compare_names_is_numeric() {
    assert_eq!(compare_names("a2", "a10"), Ordering::Less);
    assert_eq!(compare_names("B", "a"), Ordering::Greater);
    assert_eq!(compare_names("素材", "素材"), Ordering::Equal);
}
