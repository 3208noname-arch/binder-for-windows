//! クイックルックの再生のキー操作の、再生部品に依存しない計算。Eagleの詳細表示の値に合わせる。

/// 速度の段。Shift+`,` `.` で1つずつ動く。
pub const SPEEDS: [f64; 11] = [0.25, 0.5, 0.75, 1., 1.25, 1.5, 1.75, 2., 3., 4., 8.];
/// 1コマの時間が測れない時に使う値（秒）。
pub const FALLBACK_FRAME: f64 = 1. / 24.;
/// 少し送る幅の、長さに対する割合と上限（秒）。
const SKIP_SHARE: f64 = 0.02;
const SKIP_LIMIT: f64 = 10.;
/// トラックパッドの細かい回転でも、これより小さくは送らない。
const WHEEL_MIN: f64 = 0.1;

/// 今の速度から1段動いた速度。段に無い速度からは、近い側の次の段へ動く。
pub fn next_speed(current: f64, faster: bool) -> f64 {
    let next = if faster { SPEEDS.iter().find(|&&speed| speed > current + 1e-6) } else { SPEEDS.iter().rev().find(|&&speed| speed < current - 1e-6) };
    next.copied().unwrap_or(if faster { SPEEDS[SPEEDS.len() - 1] } else { SPEEDS[0] })
}

/// Ctrl+←→ とホイール1回で送る秒数。長さの2%、最大10秒。
pub fn skip_seconds(duration: f64) -> f64 {
    (duration.max(0.) * SKIP_SHARE).min(SKIP_LIMIT)
}

/// ホイールの回転量（行）を、送る幅に掛ける割合にする。1ノッチは1行以上なので丸ごと送る。
pub fn wheel_share(lines: f32) -> f64 {
    (lines.abs() as f64).clamp(WHEEL_MIN, 1.)
}

/// 速度の表示。1倍の時は出さない。
pub fn speed_label(speed: f64) -> Option<String> {
    ((speed - 1.).abs() > 1e-6).then(|| format!("{}×", (speed * 100.).round() / 100.))
}

/// 位置を長さの中に収める。終わりちょうどに置くと終了の扱いになるので、最後のコマの手前で止める。
pub fn clamp_position(seconds: f64, duration: f64, frame: f64) -> f64 {
    if duration <= 0. {
        return seconds.max(0.);
    }
    seconds.clamp(0., (duration - frame).max(0.))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_moves_one_step_and_stops_at_the_ends() {
        assert_eq!(next_speed(1., true), 1.25);
        assert_eq!(next_speed(1., false), 0.75);
        assert_eq!(next_speed(4., true), 8.);
        assert_eq!(next_speed(8., true), 8.);
        assert_eq!(next_speed(0.25, false), 0.25);
    }

    #[test]
    fn speed_off_the_table_goes_to_the_next_step() {
        assert_eq!(next_speed(1.1, true), 1.25);
        assert_eq!(next_speed(1.1, false), 1.);
    }

    #[test]
    fn skip_is_two_percent_up_to_ten_seconds() {
        assert_eq!(skip_seconds(100.), 2.);
        assert_eq!(skip_seconds(3600.), 10.);
        assert_eq!(skip_seconds(0.), 0.);
    }

    #[test]
    fn small_wheel_turns_skip_less() {
        assert_eq!(wheel_share(3.), 1.);
        assert_eq!(wheel_share(-0.5), 0.5);
        assert_eq!(wheel_share(0.01), 0.1);
    }

    #[test]
    fn speed_label_hides_normal_speed() {
        assert_eq!(speed_label(1.), None);
        assert_eq!(speed_label(1.5).as_deref(), Some("1.5×"));
        assert_eq!(speed_label(0.25).as_deref(), Some("0.25×"));
        assert_eq!(speed_label(8.).as_deref(), Some("8×"));
    }

    #[test]
    fn position_stays_before_the_last_frame() {
        assert_eq!(clamp_position(-1., 10., 0.1), 0.);
        assert_eq!(clamp_position(12., 10., 0.5), 9.5);
        assert_eq!(clamp_position(5., 10., 0.1), 5.);
    }
}
