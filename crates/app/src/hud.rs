//! Game HUD Frames (the Suite's Creators): frames round parts of a game's HUD, marked on a screenshot of it.
//! A box you draw is snapped to the element's edges: inside a margin round each side, the line where the
//! picture changes most (its edge) is where that side goes.

use serde_json::{json, Value};

/// The frames, as kept (a JSON list in the Suite's hudFrames)
pub fn frames(v: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(v).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// A new frame, at a box (the screenshot's pixels), in the style of the last one made
pub fn frame(x: f32, y: f32, w: f32, h: f32, like: Option<&Value>) -> Value {
    let style = like.and_then(|l| l["style"].as_str()).unwrap_or("brackets");
    json!({ "x": x.round(), "y": y.round(), "w": w.round().max(4.0), "h": h.round().max(4.0), "style": style, "label": "", "pad": 6, "pulse": false })
}

/// A box snapped to the element inside it: each side moved to the strongest edge within `m` pixels of it (out)
/// or a quarter of the box (in). The picture's brightness, a side's edge strength its change across that line.
pub fn snap(img: &image::GrayImage, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let px = |x: i64, y: i64| img.get_pixel(x.clamp(0, iw - 1) as u32, y.clamp(0, ih - 1) as u32).0[0] as i64;
    let (x0, y0, x1, y1) = (x as i64, y as i64, (x + w) as i64, (y + h) as i64);
    let m = 10i64;
    // (a vertical line's strength at column c, over the box's rows; a horizontal one's at row r, over its columns)
    let col = |c: i64| (y0..=y1).map(|r| (px(c, r) - px(c - 1, r)).abs()).sum::<i64>();
    let row = |r: i64| (x0..=x1).map(|c| (px(c, r) - px(c, r - 1)).abs()).sum::<i64>();
    let best = |from: i64, to: i64, f: &dyn Fn(i64) -> i64, keep: i64| {
        let (mut at, mut top) = (keep, 0i64);
        for k in from.min(to)..=from.max(to) { let e = f(k); if e > top { top = e; at = k; } }
        // (only a real edge moves it: one clearly stronger than the picture's average change along it)
        let span = (to - from).abs().max(1);
        let mean = (from.min(to)..=from.max(to)).map(f).sum::<i64>() / (span + 1);
        if top > mean * 2 { at } else { keep }
    };
    let (qw, qh) = ((x1 - x0) / 4, (y1 - y0) / 4);
    let l = best(x0 - m, x0 + qw, &col, x0);
    let r = best(x1 - qw, x1 + m, &col, x1);
    let t = best(y0 - m, y0 + qh, &row, y0);
    let b = best(y1 - qh, y1 + m, &row, y1);
    let (l, r, t, b) = (l.clamp(0, iw - 1), r.clamp(0, iw), t.clamp(0, ih - 1), b.clamp(0, ih));
    if r - l < 4 || b - t < 4 { return (x, y, w, h); }
    (l as f32, t as f32, (r - l) as f32, (b - t) as f32)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_box_snaps_to_the_square_inside_it() {
        // (a light square, 40 to 120 across and 50 to 110 down, on a dark picture; a rough box drawn round it)
        let img = image::GrayImage::from_fn(200, 160, |x, y| image::Luma([if (40..120).contains(&x) && (50..110).contains(&y) { 220 } else { 30 }]));
        let (x, y, w, h) = super::snap(&img, 34.0, 46.0, 92.0, 70.0);
        assert_eq!((x, y, x + w, y + h), (40.0, 50.0, 120.0, 110.0));
        // (on a flat picture, nothing to snap to: as drawn)
        let flat = image::GrayImage::from_pixel(200, 160, image::Luma([90]));
        assert_eq!(super::snap(&flat, 10.0, 10.0, 50.0, 40.0), (10.0, 10.0, 50.0, 40.0));
    }
}
