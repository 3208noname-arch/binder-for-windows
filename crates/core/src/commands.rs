//! 操作と、その呼び出し方の対応。操作は1つで、呼び出し方だけが増える。
//! ショートカットの設定を足す時は、`Bindings` の中身を差し替える。

/// アプリの操作。入力の種類には依存しない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    QuickLook,
    Parent,
    Back,
    Forward,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    ExtendUp,
    ExtendDown,
    ExtendLeft,
    ExtendRight,
    Open,
    Cancel,
    Search,
    Recycle,
    Rename,
    Reload,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    NewFolder,
    ToggleHidden,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    OpenInNewTab,
    EditPath,
    /// クイックルックの再生。1コマ、10コマ、長さの2%ずつ動く
    FrameBack,
    FrameForward,
    FramesBack,
    FramesForward,
    SkipBack,
    SkipForward,
    Slower,
    Faster,
    /// クイックルックの画像の拡大
    ZoomIn,
    ZoomOut,
    /// 枠に合わせると等倍を往復する
    ZoomToggle,
    ZoomActual,
    ZoomFit,
}

impl Command {
    /// キーの押しっぱなしで繰り返すか。
    pub fn repeats(self) -> bool {
        use Command::*;
        matches!(
            self,
            MoveUp | MoveDown | MoveLeft | MoveRight | ExtendUp | ExtendDown | ExtendLeft | ExtendRight | FrameBack | FrameForward | FramesBack | FramesForward | SkipBack | SkipForward | ZoomIn | ZoomOut
        )
    }

    /// 名前の入力中や検索欄の中でも効くか。
    pub fn works_while_typing(self) -> bool {
        matches!(self, Command::NewTab | Command::CloseTab | Command::NextTab | Command::PreviousTab | Command::EditPath)
    }
}

/// キーの組み合わせ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: String,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    pub fn new(key: &str, control: bool, shift: bool, alt: bool) -> Self {
        Self { key: key.to_string(), control, shift, alt }
    }
}

/// マウスの呼び出し方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    MiddleClick,
    ControlDoubleClick,
}

/// 呼び出し方から操作を引く表。
pub struct Bindings {
    keys: Vec<(Chord, Command)>,
    gestures: Vec<(Gesture, Command)>,
}

impl Default for Bindings {
    fn default() -> Self {
        use Command::*;
        const KEYS: &[(&str, bool, bool, bool, Command)] = &[
            ("space", false, false, false, QuickLook),
            ("up", false, false, true, Parent),
            ("left", false, false, true, Back),
            ("right", false, false, true, Forward),
            ("backspace", false, false, false, Back),
            ("up", false, false, false, MoveUp),
            ("down", false, false, false, MoveDown),
            ("left", false, false, false, MoveLeft),
            ("right", false, false, false, MoveRight),
            ("up", false, true, false, ExtendUp),
            ("down", false, true, false, ExtendDown),
            ("left", false, true, false, ExtendLeft),
            ("right", false, true, false, ExtendRight),
            ("enter", false, false, false, Open),
            ("escape", false, false, false, Cancel),
            ("f", true, false, false, Search),
            ("delete", false, false, false, Recycle),
            ("f2", false, false, false, Rename),
            ("f5", false, false, false, Reload),
            ("a", true, false, false, SelectAll),
            ("c", true, false, false, Copy),
            ("x", true, false, false, Cut),
            ("v", true, false, false, Paste),
            ("z", true, false, false, Undo),
            ("n", true, true, false, NewFolder),
            // gpuiは記号を打つShiftを修飾から外すので、Shift+ピリオドは「>」で届く
            (">", true, false, false, ToggleHidden),
            (".", true, true, false, ToggleHidden),
            ("l", true, false, false, EditPath),
            ("t", true, false, false, NewTab),
            ("w", true, false, false, CloseTab),
            ("tab", true, false, false, NextTab),
            ("tab", true, true, false, PreviousTab),
            // gpuiはShiftで打つ記号を、その記号として届ける（Shift+[ は「{」）
            ("[", false, false, false, FrameBack),
            ("]", false, false, false, FrameForward),
            ("{", false, false, false, FramesBack),
            ("}", false, false, false, FramesForward),
            ("left", true, false, false, SkipBack),
            ("right", true, false, false, SkipForward),
            ("<", false, false, false, Slower),
            (">", false, false, false, Faster),
            ("+", false, false, false, ZoomIn),
            ("+", true, false, false, ZoomIn),
            ("=", true, false, false, ZoomIn),
            ("-", false, false, false, ZoomOut),
            ("-", true, false, false, ZoomOut),
            ("`", false, false, false, ZoomToggle),
            ("0", true, false, false, ZoomActual),
            ("9", true, false, false, ZoomFit),
        ];
        let keys = KEYS.iter().map(|&(key, control, shift, alt, command)| (Chord::new(key, control, shift, alt), command)).collect();
        let gestures = vec![(Gesture::MiddleClick, OpenInNewTab), (Gesture::ControlDoubleClick, OpenInNewTab)];
        Self { keys, gestures }
    }
}

impl Bindings {
    pub fn key(&self, chord: &Chord) -> Option<Command> {
        self.keys.iter().find(|(bound, _)| bound == chord).map(|(_, command)| *command)
    }

    pub fn gesture(&self, gesture: Gesture) -> Option<Command> {
        self.gestures.iter().find(|(bound, _)| *bound == gesture).map(|(_, command)| *command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_mouse_gestures_run_the_same_command() {
        let bindings = Bindings::default();
        assert_eq!(bindings.gesture(Gesture::MiddleClick), Some(Command::OpenInNewTab));
        assert_eq!(bindings.gesture(Gesture::ControlDoubleClick), Some(Command::OpenInNewTab));
    }

    #[test]
    fn modifiers_choose_the_command() {
        let bindings = Bindings::default();
        assert_eq!(bindings.key(&Chord::new("up", false, false, false)), Some(Command::MoveUp));
        assert_eq!(bindings.key(&Chord::new("up", false, true, false)), Some(Command::ExtendUp));
        assert_eq!(bindings.key(&Chord::new("up", false, false, true)), Some(Command::Parent));
        assert_eq!(bindings.key(&Chord::new("tab", true, true, false)), Some(Command::PreviousTab));
        assert_eq!(bindings.key(&Chord::new("t", false, false, false)), None);
    }

    #[test]
    fn one_command_can_have_several_keys() {
        let bindings = Bindings::default();
        assert_eq!(bindings.key(&Chord::new("backspace", false, false, false)), Some(Command::Back));
        assert_eq!(bindings.key(&Chord::new("left", false, false, true)), Some(Command::Back));
        assert_eq!(bindings.key(&Chord::new(">", true, false, false)), Some(Command::ToggleHidden));
        assert_eq!(bindings.key(&Chord::new(".", true, true, false)), Some(Command::ToggleHidden));
    }

    #[test]
    fn no_chord_is_bound_twice() {
        let bindings = Bindings::default();
        for (index, (chord, _)) in bindings.keys.iter().enumerate() {
            assert!(bindings.keys[index + 1..].iter().all(|(other, _)| other != chord), "{chord:?}");
        }
    }

    #[test]
    fn only_tab_commands_work_while_typing() {
        assert!(Command::CloseTab.works_while_typing());
        assert!(!Command::Paste.works_while_typing());
    }

    #[test]
    fn space_opens_quick_look_only_outside_typing() {
        let bindings = Bindings::default();
        assert_eq!(bindings.key(&Chord::new("space", false, false, false)), Some(Command::QuickLook));
        assert!(!Command::QuickLook.works_while_typing());
        assert!(!Command::QuickLook.repeats());
    }

    #[test]
    fn quick_look_keys_follow_eagle() {
        let bindings = Bindings::default();
        let key = |key: &str, control: bool| bindings.key(&Chord::new(key, control, false, false));
        assert_eq!(key("]", false), Some(Command::FrameForward));
        assert_eq!(key("{", false), Some(Command::FramesBack));
        assert_eq!(key("left", true), Some(Command::SkipBack));
        assert_eq!(key(">", false), Some(Command::Faster));
        assert_eq!(key(">", true), Some(Command::ToggleHidden));
        assert_eq!(key("`", false), Some(Command::ZoomToggle));
        assert_eq!(key("0", true), Some(Command::ZoomActual));
        assert!(Command::FrameForward.repeats());
        assert!(!Command::Faster.repeats());
    }

    #[test]
    fn only_movement_repeats() {
        assert!(Command::ExtendLeft.repeats());
        assert!(!Command::NextTab.repeats());
    }
}
