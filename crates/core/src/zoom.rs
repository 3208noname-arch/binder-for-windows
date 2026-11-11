//! クイックルックの画像の拡大の計算。倍率は「元の1画素が画面の何画素になるか」で表す。

/// 拡大の段。`+` `-` とCtrl+ホイールで1つずつ動く。
pub const ZOOM_LEVELS: [f32; 8] = [0.05, 0.1, 0.25, 0.5, 1., 2., 4., 8.];

/// 今の倍率から1段動いた倍率。段に無い倍率（枠に合わせた時など）からは、その向きの次の段へ動く。
pub fn zoom_step(current: f32, zoom_in: bool) -> f32 {
    let next = if zoom_in { ZOOM_LEVELS.iter().find(|&&level| level > current * 1.001) } else { ZOOM_LEVELS.iter().rev().find(|&&level| level < current * 0.999) };
    next.copied().unwrap_or(if zoom_in { ZOOM_LEVELS[ZOOM_LEVELS.len() - 1] } else { ZOOM_LEVELS[0] })
}

/// 枠に合わせた倍率。元の大きさより大きくはしない。
pub fn fit_scale(image: (f32, f32), view: (f32, f32)) -> f32 {
    if image.0 <= 0. || image.1 <= 0. || view.0 <= 0. || view.1 <= 0. {
        return 1.;
    }
    (view.0 / image.0).min(view.1 / image.1).min(1.)
}

/// 画像の左上の位置を、枠の中に収める。枠より小さい向きは中央に置き、大きい向きは端に隙間を作らない。
pub fn clamp_offset(offset: f32, image: f32, view: f32) -> f32 {
    if image <= view { (view - image) / 2. } else { offset.clamp(view - image, 0.) }
}

/// 倍率を変えた後の左上の位置。軸の点の下にある画像の場所を動かさない。
pub fn zoom_offset(offset: f32, anchor: f32, old_scale: f32, new_scale: f32) -> f32 {
    anchor - (anchor - offset) * new_scale / old_scale
}

/// 倍率の表示。小数は丸める。
pub fn zoom_label(scale: f32) -> String {
    format!("{}%", (scale * 100.).round())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_move_through_the_levels_and_stop_at_the_ends() {
        assert_eq!(zoom_step(1., true), 2.);
        assert_eq!(zoom_step(1., false), 0.5);
        assert_eq!(zoom_step(8., true), 8.);
        assert_eq!(zoom_step(0.05, false), 0.05);
    }

    #[test]
    fn fitted_scale_goes_to_the_next_level() {
        assert_eq!(zoom_step(0.37, true), 0.5);
        assert_eq!(zoom_step(0.37, false), 0.25);
    }

    #[test]
    fn fit_never_enlarges() {
        assert_eq!(fit_scale((4000., 3000.), (800., 600.)), 0.2);
        assert_eq!(fit_scale((100., 50.), (800., 600.)), 1.);
        assert_eq!(fit_scale((1000., 3000.), (800., 600.)), 0.2);
    }

    #[test]
    fn small_image_is_centered_and_large_image_leaves_no_gap() {
        assert_eq!(clamp_offset(-50., 400., 800.), 200.);
        assert_eq!(clamp_offset(30., 1600., 800.), 0.);
        assert_eq!(clamp_offset(-1000., 1600., 800.), -800.);
        assert_eq!(clamp_offset(-300., 1600., 800.), -300.);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_anchor() {
        // 枠の400の位置は画像の200の位置。2倍にしても400の下は画像の200のまま
        let offset = zoom_offset(200., 400., 1., 2.);
        assert_eq!((400. - offset) / 2., 200.);
    }

    #[test]
    fn label_is_a_rounded_percent() {
        assert_eq!(zoom_label(0.25), "25%");
        assert_eq!(zoom_label(0.3333), "33%");
    }
}
