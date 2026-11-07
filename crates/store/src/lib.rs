//! 設定をSQLite 1ファイルへ保存する部品。

use filer_core::{AppSettings, Favorite, FolderView, TabSession};
use std::path::Path;

/// フォルダごとの表示の設定を持つ。
pub struct Store {
    connection: rusqlite::Connection,
}

impl Store {
    pub fn open(file: &Path) -> rusqlite::Result<Self> {
        let connection = rusqlite::Connection::open(file)?;
        // 画面の操作ごとに書くので、待ちを短くする
        connection.busy_timeout(std::time::Duration::from_millis(200))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS folder_view (path TEXT PRIMARY KEY, view TEXT NOT NULL)",
            [],
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS app_settings (id INTEGER PRIMARY KEY CHECK (id = 1), value TEXT NOT NULL)",
            [],
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS tab_session (id INTEGER PRIMARY KEY CHECK (id = 1), value TEXT NOT NULL)",
            [],
        )?;
        connection.execute(
            "CREATE TABLE IF NOT EXISTS favorites (id INTEGER PRIMARY KEY CHECK (id = 1), value TEXT NOT NULL)",
            [],
        )?;
        Ok(Self { connection })
    }

    /// アプリ全体の設定。保存が無ければ既定の値。
    pub fn app_settings(&self) -> AppSettings {
        let saved: Option<String> =
            self.connection.query_row("SELECT value FROM app_settings WHERE id = 1", [], |row| row.get(0)).ok();
        saved.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save_app_settings(&self, settings: &AppSettings) -> rusqlite::Result<()> {
        let text = serde_json::to_string(settings).expect("設定は必ず文字列にできる");
        self.connection.execute(
            "INSERT INTO app_settings (id, value) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET value = excluded.value",
            [text],
        )?;
        Ok(())
    }

    /// 前回のタブ。保存が無い時と、壊れていて読めない時は `None`。
    pub fn tab_session(&self) -> Option<TabSession> {
        let text: String = self.connection.query_row("SELECT value FROM tab_session WHERE id = 1", [], |row| row.get(0)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_tab_session(&self, session: &TabSession) -> rusqlite::Result<()> {
        let text = serde_json::to_string(session).expect("タブの並びは必ず文字列にできる");
        self.connection.execute(
            "INSERT INTO tab_session (id, value) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET value = excluded.value",
            [text],
        )?;
        Ok(())
    }

    /// よく使う項目の並び。一度も変えていない時と、壊れていて読めない時は `None`。
    /// 全部外した時は空の並びが返るので、既定の項目へは戻らない。
    pub fn favorites(&self) -> Option<Vec<Favorite>> {
        let text: String = self.connection.query_row("SELECT value FROM favorites WHERE id = 1", [], |row| row.get(0)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_favorites(&self, favorites: &[Favorite]) -> rusqlite::Result<()> {
        let text = serde_json::to_string(favorites).expect("よく使う項目は必ず文字列にできる");
        self.connection.execute(
            "INSERT INTO favorites (id, value) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET value = excluded.value",
            [text],
        )?;
        Ok(())
    }

    /// 保存が無い時と、壊れていて読めない時は `None`。
    pub fn folder_view(&self, folder: &Path) -> Option<FolderView> {
        let text: String = self
            .connection
            .query_row("SELECT view FROM folder_view WHERE path = ?1", [key(folder)], |row| row.get(0))
            .ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save_folder_view(&self, folder: &Path, view: &FolderView) -> rusqlite::Result<()> {
        let text = serde_json::to_string(view).expect("表示の設定は必ず文字列にできる");
        self.connection.execute(
            "INSERT INTO folder_view (path, view) VALUES (?1, ?2)
             ON CONFLICT (path) DO UPDATE SET view = excluded.view",
            [key(folder), text],
        )?;
        Ok(())
    }
}

/// Windowsのパスは大文字と小文字を区別しない。
fn key(folder: &Path) -> String {
    folder.to_string_lossy().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use filer_core::{GroupKey, SavedTab, ViewMode};

    #[test]
    fn favorites_come_back_after_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.sqlite");
        let favorites = vec![
            Favorite { name: "仕事".into(), path: r"\\nas\共有\素材".into() },
            Favorite::new(r"C:\もう無いフォルダ".into()),
        ];
        Store::open(&file).unwrap().save_favorites(&favorites).unwrap();
        assert_eq!(Store::open(&file).unwrap().favorites(), Some(favorites));
    }

    #[test]
    fn an_emptied_favorites_list_stays_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        store.save_favorites(&[]).unwrap();
        assert_eq!(store.favorites(), Some(Vec::new()));
    }

    #[test]
    fn missing_or_broken_favorites_are_treated_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        assert_eq!(store.favorites(), None);
        store.connection.execute("INSERT INTO favorites VALUES (1, 'not json')", []).unwrap();
        assert_eq!(store.favorites(), None);
    }

    #[test]
    fn opening_an_old_file_adds_the_favorites_table() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.sqlite");
        rusqlite::Connection::open(&file).unwrap().execute("CREATE TABLE app_settings (id INTEGER PRIMARY KEY, value TEXT)", []).unwrap();
        assert_eq!(Store::open(&file).unwrap().favorites(), None);
    }

    #[test]
    fn tab_session_comes_back_after_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.sqlite");
        let tabs = vec![
            SavedTab { path: r"C:\素材".into(), mode: ViewMode::Columns },
            SavedTab { path: r"D:\".into(), mode: ViewMode::Icons },
        ];
        let session = TabSession { tabs, active: 1 };
        Store::open(&file).unwrap().save_tab_session(&session).unwrap();
        assert_eq!(Store::open(&file).unwrap().tab_session(), Some(session));
    }

    #[test]
    fn saving_tabs_twice_keeps_the_latest() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        store.save_tab_session(&TabSession::default()).unwrap();
        let session = TabSession { tabs: vec![SavedTab { path: "a".into(), mode: ViewMode::List }], active: 0 };
        store.save_tab_session(&session).unwrap();
        assert_eq!(store.tab_session(), Some(session));
    }

    #[test]
    fn missing_or_broken_tab_session_is_treated_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        assert_eq!(store.tab_session(), None);
        store.connection.execute("INSERT INTO tab_session VALUES (1, 'not json')", []).unwrap();
        assert_eq!(store.tab_session(), None);
    }

    #[test]
    fn app_settings_come_back_after_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.sqlite");
        let settings = AppSettings { show_hidden: true, ..AppSettings::default() };
        Store::open(&file).unwrap().save_app_settings(&settings).unwrap();
        assert_eq!(Store::open(&file).unwrap().app_settings(), settings);
    }

    #[test]
    fn missing_app_settings_use_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        assert_eq!(store.app_settings(), AppSettings::default());
    }

    #[test]
    fn saved_view_comes_back_after_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.sqlite");
        let view = FolderView { mode: ViewMode::Icons, group: GroupKey::Size, ..FolderView::default() };
        Store::open(&file).unwrap().save_folder_view(Path::new(r"C:\素材"), &view).unwrap();
        assert_eq!(Store::open(&file).unwrap().folder_view(Path::new(r"C:\素材")), Some(view));
    }

    #[test]
    fn unknown_folder_has_no_view() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        assert_eq!(store.folder_view(Path::new(r"C:\none")), None);
    }

    #[test]
    fn folder_paths_ignore_case() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        store.save_folder_view(Path::new(r"C:\Users"), &FolderView::default()).unwrap();
        assert!(store.folder_view(Path::new(r"c:\users")).is_some());
    }

    #[test]
    fn saving_twice_keeps_the_latest() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        let path = Path::new(r"C:\a");
        store.save_folder_view(path, &FolderView::default()).unwrap();
        let view = FolderView { group: GroupKey::Name, ..FolderView::default() };
        store.save_folder_view(path, &view).unwrap();
        assert_eq!(store.folder_view(path), Some(view));
    }

    #[test]
    fn broken_row_is_treated_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("s.sqlite")).unwrap();
        store.connection.execute("INSERT INTO folder_view VALUES ('c:\\x', 'not json')", []).unwrap();
        assert_eq!(store.folder_view(Path::new(r"C:\x")), None);
    }
}
