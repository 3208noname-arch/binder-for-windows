//! 選択済みの項目の名前をもう1回クリックした時に、名前の変更を始める判定。Explorer と同じ動き。
//!
//! 押した時に始めてよいかを決め（`press`）、離した時に待ちに入り（`release`）、
//! ダブルクリックの判定時間が過ぎてもまだ同じ状態なら始める（`fire`）。
//! 間に別のクリックやドラッグが来たら `cancel` で取り消す。

use std::path::{Path, PathBuf};

/// 名前の上を押した時の状況。
pub struct Press<'a> {
    pub path: &'a Path,
    /// 押す前から、この項目だけが選ばれていたか。
    pub sole_selected: bool,
    pub click_count: usize,
    /// Ctrl、Shift、Alt のどれかが押されていたか。
    pub modified: bool,
    /// ウィンドウにフォーカスが無い状態からのクリックか。
    pub from_inactive: bool,
}

#[derive(Default)]
pub struct SlowClick {
    armed: Option<PathBuf>,
    pending: Option<(PathBuf, u64)>,
    next: u64,
}

impl SlowClick {
    /// 名前の上でボタンを押した。前の待ちは取り消す。
    pub fn press(&mut self, press: Press) {
        self.pending = None;
        let starts = press.sole_selected && press.click_count == 1 && !press.modified && !press.from_inactive;
        self.armed = starts.then(|| press.path.to_path_buf());
    }

    /// どこかでボタンを押した、ドラッグが始まった、など。始めるのをやめる。
    pub fn cancel(&mut self) {
        self.armed = None;
        self.pending = None;
    }

    /// 名前の上でボタンを離した。待ちに入るなら、時間が来た時に `fire` へ渡す番号を返す。
    pub fn release(&mut self, path: &Path, click_count: usize, dragged: bool) -> Option<u64> {
        let armed = self.armed.take()?;
        if armed != path || click_count != 1 || dragged {
            return None;
        }
        self.next += 1;
        self.pending = Some((armed, self.next));
        Some(self.next)
    }

    /// 待ち時間が過ぎた。`sole_selected` は今この項目だけが選ばれている時のそのパス。
    /// 名前の変更を始める項目を返す。
    pub fn fire(&mut self, token: u64, sole_selected: Option<&Path>) -> Option<PathBuf> {
        let (path, _) = self.pending.take_if(|(_, pending)| *pending == token)?;
        (sole_selected == Some(path.as_path())).then_some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(path: &Path) -> Press<'_> {
        Press { path, sole_selected: true, click_count: 1, modified: false, from_inactive: false }
    }

    #[test]
    fn second_slow_click_on_selected_name_starts_rename() {
        let path = Path::new(r"C:\a.txt");
        let mut click = SlowClick::default();
        click.press(press(path));
        let token = click.release(path, 1, false).unwrap();
        assert_eq!(click.fire(token, Some(path)).as_deref(), Some(path));
    }

    #[test]
    fn first_click_on_unselected_item_only_selects() {
        let path = Path::new(r"C:\a.txt");
        let mut click = SlowClick::default();
        click.press(Press { sole_selected: false, ..press(path) });
        assert_eq!(click.release(path, 1, false), None);
    }

    #[test]
    fn modifiers_multi_selection_and_inactive_window_do_not_start() {
        let path = Path::new(r"C:\a.txt");
        for variant in [
            Press { modified: true, ..press(path) },
            Press { from_inactive: true, ..press(path) },
            // 複数選択中は「この項目だけが選ばれている」にならない
            Press { sole_selected: false, ..press(path) },
        ] {
            let mut click = SlowClick::default();
            click.press(variant);
            assert_eq!(click.release(path, 1, false), None);
        }
    }

    #[test]
    fn double_click_cancels_the_wait() {
        let path = Path::new(r"C:\a.txt");
        let mut click = SlowClick::default();
        click.press(press(path));
        let token = click.release(path, 1, false).unwrap();
        // 2回目の押下はまずウィンドウ全体で取り消され、名前の上でも2回目として扱われる
        click.cancel();
        click.press(Press { click_count: 2, ..press(path) });
        assert_eq!(click.release(path, 2, false), None);
        assert_eq!(click.fire(token, Some(path)), None);
    }

    #[test]
    fn drag_cancels() {
        let path = Path::new(r"C:\a.txt");
        let mut click = SlowClick::default();
        click.press(press(path));
        assert_eq!(click.release(path, 1, true), None);

        click.press(press(path));
        click.cancel();
        assert_eq!(click.release(path, 1, false), None);
    }

    #[test]
    fn selection_change_during_wait_cancels() {
        let (path, other) = (Path::new(r"C:\a.txt"), Path::new(r"C:\b.txt"));
        let mut click = SlowClick::default();
        click.press(press(path));
        let token = click.release(path, 1, false).unwrap();
        assert_eq!(click.fire(token, Some(other)), None);

        click.press(press(path));
        let token = click.release(path, 1, false).unwrap();
        assert_eq!(click.fire(token, None), None);
    }

    #[test]
    fn released_on_another_item_does_not_start() {
        let (path, other) = (Path::new(r"C:\a.txt"), Path::new(r"C:\b.txt"));
        let mut click = SlowClick::default();
        click.press(press(path));
        assert_eq!(click.release(other, 1, false), None);
    }

    #[test]
    fn old_timer_does_not_fire_a_newer_wait() {
        let path = Path::new(r"C:\a.txt");
        let mut click = SlowClick::default();
        click.press(press(path));
        let old = click.release(path, 1, false).unwrap();
        click.press(press(path));
        let new = click.release(path, 1, false).unwrap();
        assert_eq!(click.fire(old, Some(path)), None);
        assert_eq!(click.fire(new, Some(path)).as_deref(), Some(path));
    }
}
