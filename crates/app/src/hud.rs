//! Game HUD Frames (the Suite's Creators): frames round parts of a game's HUD, marked on a screenshot of it.
//! A box you draw is snapped to the element's edges: inside a margin round each side, the line where the
//! picture changes most (its edge) is where that side goes.

use serde_json::{json, Value};

/// The frames, as kept (a JSON list in the Suite's hudFrames)
pub fn frames(v: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(v).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
}

/// What makes a frame's look, besides its style: carried from one frame to the next made, and to all by Same Look for All
pub const LOOK_KEYS: [&str; 6] = ["anim", "thick", "round", "fill", "labelStyle", "pulse"];

/// A new frame, at a box (the screenshot's pixels), in the style of the last one made
pub fn frame(x: f32, y: f32, w: f32, h: f32, like: Option<&Value>) -> Value {
    let style = like.and_then(|l| l["style"].as_str()).unwrap_or("brackets");
    let mut f = json!({ "x": x.round(), "y": y.round(), "w": w.round().max(4.0), "h": h.round().max(4.0), "style": style, "label": "", "pad": 6, "pulse": false });
    // (the rest of the last one's look too, so a set of frames matches without setting each)
    if let Some(l) = like { for k in LOOK_KEYS { if !l[k].is_null() { f[k] = l[k].clone(); } } }
    f
}

/// A frame fitted to the element in a box you drew (the screenshot's pixels): wrapped in its own shape if that
/// isn't a box (traced: "auto"), else the box snapped to its edges ("rect")
pub fn fit(rgb: &image::RgbImage, gray: &image::GrayImage, f: &mut Value, x: f32, y: f32, w: f32, h: f32) {
    let (bx, by, bw, bh) = match wrap(rgb, x, y, w, h).filter(|r| !boxy(&r.4)) {
        Some((bx, by, bw, bh, pts)) => { f["shape"] = json!("auto"); f["pts"] = json!(pts.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>()); (bx, by, bw, bh) }
        None => { f["shape"] = json!("rect"); if let Some(o) = f.as_object_mut() { o.remove("pts"); } snap(gray, x, y, w, h) }
    };
    f["x"] = json!(bx.round()); f["y"] = json!(by.round()); f["w"] = json!(bw.round().max(4.0)); f["h"] = json!(bh.round().max(4.0));
}

/// A frame given a shape: traced from the screenshot again ("auto"; a box if there's nothing to trace), its box
/// ("rect"), or one of the others, in its box as it is
pub fn reshape(rgb: Option<&image::RgbImage>, f: &mut Value, shape: &str) {
    let a = |k: &str| f[k].as_f64().unwrap_or(0.0) as f32;
    let (x, y, w, h) = (a("x"), a("y"), a("w"), a("h"));
    let pts = match shape {
        "auto" => match rgb.and_then(|img| wrap(img, x, y, w, h)) {
            Some((bx, by, bw, bh, pts)) => { f["x"] = json!(bx.round()); f["y"] = json!(by.round()); f["w"] = json!(bw.round().max(4.0)); f["h"] = json!(bh.round().max(4.0)); pts }
            None => Vec::new(),
        },
        s => shape_points(s, w, h),
    };
    f["shape"] = json!(if shape == "auto" && pts.is_empty() { "rect" } else { shape });
    if pts.is_empty() { if let Some(o) = f.as_object_mut() { o.remove("pts"); } } else { f["pts"] = json!(pts.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>()); }
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

/// The shapes a frame can take (key, name): its box as it is, the element's own outline traced from the
/// screenshot, or one of these, drawn inside its box
pub const SHAPES: [(&str, &str); 8] = [("rect", "Box"), ("auto", "Traced"), ("slant-r", "Slanted /"), ("slant-l", "Slanted \\"), ("tri-down", "Triangle ▽"), ("tri-up", "Triangle △"), ("hex", "Hexagon"), ("cut", "Cut Corners")];

/// A shape's corners, as fractions of the frame's box (none: the box itself)
pub fn shape_points(shape: &str, w: f32, h: f32) -> Vec<(f32, f32)> {
    // (a slant as steep as the HUDs' usually are: about a fifth of its height across, whatever its width)
    let s = (h * 0.36 / w.max(1.0)).clamp(0.06, 0.3);
    // (cut corners: the same size each way, a sixth of its shorter side)
    let (cx, cy) = (w.min(h) / 6.0 / w.max(1.0), w.min(h) / 6.0 / h.max(1.0));
    match shape {
        "slant-r" => vec![(s, 0.0), (1.0, 0.0), (1.0 - s, 1.0), (0.0, 1.0)],
        "slant-l" => vec![(0.0, 0.0), (1.0 - s, 0.0), (1.0, 1.0), (s, 1.0)],
        "tri-down" => vec![(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)],
        "tri-up" => vec![(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)],
        "hex" => vec![(0.22, 0.0), (0.78, 0.0), (1.0, 0.5), (0.78, 1.0), (0.22, 1.0), (0.0, 0.5)],
        "cut" => vec![(cx, 0.0), (1.0, 0.0), (1.0, 1.0 - cy), (1.0 - cx, 1.0), (0.0, 1.0), (0.0, cy)],
        _ => Vec::new(),
    }
}

/// The element inside a box you drew, wrapped: its outline found (what's unlike the colour round the box's edge,
/// the biggest such patch near the middle), the convex shape round it, and that cut down to its few main corners
/// (at most 8; fewer while that keeps nearly all of it). Its box (the screenshot's pixels) and its corners (as
/// fractions of the box); none if nothing stands out there
pub fn wrap(img: &image::RgbImage, x: f32, y: f32, w: f32, h: f32) -> Option<(f32, f32, f32, f32, Vec<(f32, f32)>)> {
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let m = 8i64;
    let (x0, y0) = ((x as i64 - m).clamp(0, iw - 1), (y as i64 - m).clamp(0, ih - 1));
    let (x1, y1) = (((x + w) as i64 + m).clamp(x0 + 1, iw), ((y + h) as i64 + m).clamp(y0 + 1, ih));
    let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if rw < 6 || rh < 6 { return None; }
    let at = |cx: usize, cy: usize| img.get_pixel((x0 as usize + cx) as u32, (y0 as usize + cy) as u32).0;
    // (the background: the middle colour along the region's edge, and how much it varies there)
    let mut edge = Vec::new();
    for cx in 0..rw { edge.push(at(cx, 0)); edge.push(at(cx, rh - 1)); }
    for cy in 0..rh { edge.push(at(0, cy)); edge.push(at(rw - 1, cy)); }
    let med = |k: usize| { let mut v: Vec<u8> = edge.iter().map(|p| p[k]).collect(); v.sort_unstable(); v[v.len() / 2] as f32 };
    let bg = [med(0), med(1), med(2)];
    let dist = |p: [u8; 3]| ((p[0] as f32 - bg[0]).powi(2) + (p[1] as f32 - bg[1]).powi(2) + (p[2] as f32 - bg[2]).powi(2)).sqrt();
    let mut spread: Vec<f32> = edge.iter().map(|p| dist(*p)).collect();
    spread.sort_by(f32::total_cmp);
    let t = (spread[spread.len() / 2] * 3.0 + 14.0).max(34.0);
    let mask: Vec<bool> = (0..rh).flat_map(|cy| (0..rw).map(move |cx| (cx, cy))).map(|(cx, cy)| dist(at(cx, cy)) > t).collect();
    // (its patches: the biggest one reaching the middle half of the box drawn)
    let mut label = vec![0u32; rw * rh];
    let mut best: Option<(u32, usize)> = None;
    // (and the others that are in the box you drew (most of each inside it, and not specks): a frame round several
    // parts takes them all)
    let mut also: Vec<u32> = Vec::new();
    let (ix0, iy0, ix1, iy1) = ((x - x0 as f32).max(0.0) as usize, (y - y0 as f32).max(0.0) as usize, (x - x0 as f32 + w) as usize, (y - y0 as f32 + h) as usize);
    let (mx0, my0, mx1, my1) = ((x - x0 as f32 + w * 0.25) as usize, (y - y0 as f32 + h * 0.25) as usize, (x - x0 as f32 + w * 0.75) as usize, (y - y0 as f32 + h * 0.75) as usize);
    let mut next = 0u32;
    for start in 0..rw * rh {
        if !mask[start] || label[start] != 0 { continue; }
        next += 1;
        let (mut size, mut middle, mut inside) = (0usize, false, 0usize);
        let mut stack = vec![start];
        label[start] = next;
        while let Some(i) = stack.pop() {
            size += 1;
            let (cx, cy) = (i % rw, i / rw);
            if (mx0..=mx1).contains(&cx) && (my0..=my1).contains(&cy) { middle = true; }
            if (ix0..=ix1).contains(&cx) && (iy0..=iy1).contains(&cy) { inside += 1; }
            let mut push = |j: usize| if mask[j] && label[j] == 0 { label[j] = next; stack.push(j); };
            if cx > 0 { push(i - 1); }
            if cx + 1 < rw { push(i + 1); }
            if cy > 0 { push(i - rw); }
            if cy + 1 < rh { push(i + rw); }
        }
        if middle && best.is_none_or(|b| size > b.1) { best = Some((next, size)); }
        if inside * 10 >= size * 7 && size as f32 >= (0.004 * w * h).max(12.0) { also.push(next); }
    }
    let (id, size) = best?;
    if (size as f32) < 0.03 * w * h { return None; }
    // (its outline: each row's ends, as pixel corners, then the convex shape round them)
    let mut pts: Vec<(f32, f32)> = Vec::new();
    for cy in 0..rh {
        let row = &label[cy * rw..(cy + 1) * rw];
        let ours = |v: &u32| *v == id || also.contains(v);
        if let (Some(l), Some(r)) = (row.iter().position(|v| ours(v)), row.iter().rposition(|v| ours(v))) {
            for (px, py) in [(l as f32, cy as f32), (r as f32 + 1.0, cy as f32), (l as f32, cy as f32 + 1.0), (r as f32 + 1.0, cy as f32 + 1.0)] { pts.push((px + x0 as f32, py + y0 as f32)); }
        }
    }
    let mut hull = convex_hull(pts);
    if hull.len() < 3 { return None; }
    // (cut down to its main corners: the corner that adds least taken away, while it adds under 3% (or there
    // are more than 8), keeping at least 3)
    let area = |p: &[(f32, f32)]| (0..p.len()).map(|i| { let (a, b) = (p[i], p[(i + 1) % p.len()]); a.0 * b.1 - b.0 * a.1 }).sum::<f32>().abs() / 2.0;
    let whole = area(&hull);
    loop {
        if hull.len() <= 3 { break; }
        let n = hull.len();
        let tri = |i: usize| { let (a, b, c) = (hull[(i + n - 1) % n], hull[i], hull[(i + 1) % n]); ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs() / 2.0 };
        let (i, least) = (0..n).map(|i| (i, tri(i))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((0, 0.0));
        if n > 12 || least < whole * 0.012 && whole - area(&hull) + least < whole * 0.03 { hull.remove(i); } else { break; }
    }
    let (bx0, by0) = hull.iter().fold((f32::MAX, f32::MAX), |a, p| (a.0.min(p.0), a.1.min(p.1)));
    let (bx1, by1) = hull.iter().fold((f32::MIN, f32::MIN), |a, p| (a.0.max(p.0), a.1.max(p.1)));
    let (bw, bh) = ((bx1 - bx0).max(1.0), (by1 - by0).max(1.0));
    let norm: Vec<(f32, f32)> = hull.iter().map(|p| (((p.0 - bx0) / bw * 1000.0).round() / 1000.0, ((p.1 - by0) / bh * 1000.0).round() / 1000.0)).collect();
    Some((bx0, by0, bw, bh, norm))
}

/// A frame's corners (as fractions of its box), as kept in it ("pts"); none for a box
pub fn points_of(f: &Value) -> Vec<(f32, f32)> {
    f["pts"].as_array().map(|a| a.iter().filter_map(|p| Some((p[0].as_f64()? as f32, p[1].as_f64()? as f32))).collect()).unwrap_or_default()
}

/// A shaped frame's lines, for drawing on its box with the room round it (`w` × `h`, its corners `pts` as fractions
/// of that): its outline, the same a little inside it (Neon's second line), and short lines along each edge from
/// each corner (Corner Brackets'), as SVG path data in that box's pixels
pub fn shape_paths(pts: &[(f32, f32)], w: f32, h: f32) -> (String, String, String) {
    let p: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (x * w, y * h)).collect();
    if p.len() < 3 { return Default::default(); }
    let path = |q: &[(f32, f32)]| q.iter().enumerate().map(|(i, (x, y))| format!("{}{x:.1} {y:.1}", if i == 0 { "M" } else { " L" })).collect::<String>() + " Z";
    let (cx, cy) = (p.iter().map(|q| q.0).sum::<f32>() / p.len() as f32, p.iter().map(|q| q.1).sum::<f32>() / p.len() as f32);
    let k = (1.0 - 10.0 / w.min(h).max(20.0)).max(0.5);
    let inner: Vec<(f32, f32)> = p.iter().map(|&(x, y)| (cx + (x - cx) * k, cy + (y - cy) * k)).collect();
    let reach = w.min(h) * 0.24;
    let mut br = String::new();
    for i in 0..p.len() {
        let (a, b, c) = (p[(i + p.len() - 1) % p.len()], p[i], p[(i + 1) % p.len()]);
        let toward = |o: (f32, f32)| { let (dx, dy) = (o.0 - b.0, o.1 - b.1); let l = (dx * dx + dy * dy).sqrt().max(1.0); let r = reach.min(l * 0.4); (b.0 + dx / l * r, b.1 + dy / l * r) };
        let (s, e) = (toward(a), toward(c));
        br.push_str(&format!("M{:.1} {:.1} L{:.1} {:.1} L{:.1} {:.1} ", s.0, s.1, b.0, b.1, e.0, e.1));
    }
    (path(&p), path(&inner), br.trim_end().to_string())
}

/// Whether corners (as fractions of their box) make nearly the whole box: a rectangle
pub fn boxy(pts: &[(f32, f32)]) -> bool {
    let a = (0..pts.len()).map(|i| { let (p, q) = (pts[i], pts[(i + 1) % pts.len()]); p.0 * q.1 - q.0 * p.1 }).sum::<f32>().abs() / 2.0;
    pts.len() < 3 || a > 0.93
}

/// The convex shape round some points (Andrew's monotone chain), clockwise on screen
fn convex_hull(mut p: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    p.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    p.dedup();
    if p.len() < 3 { return p; }
    let cross = |o: (f32, f32), a: (f32, f32), b: (f32, f32)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<(f32, f32)> = Vec::new();
    for &q in &p { while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 { lower.pop(); } lower.push(q); }
    let mut upper: Vec<(f32, f32)> = Vec::new();
    for &q in p.iter().rev() { while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 { upper.pop(); } upper.push(q); }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
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

    #[test]
    fn slanted_and_triangular_parts_are_wrapped_in_their_shape() {
        // (a dark slanted block (like a game's ability slot) on a light floor, and a triangle, each a rough box round it)
        let slant = |x: f32, y: f32| (40.0..=100.0).contains(&y) && x >= 60.0 + (100.0 - y) * 0.3 && x <= 140.0 + (100.0 - y) * 0.3;
        let img = image::RgbImage::from_fn(240, 160, |x, y| image::Rgb(if slant(x as f32, y as f32) { [30, 34, 44] } else { [196, 190, 176] }));
        let (x, y, w, h, pts) = super::wrap(&img, 50.0, 30.0, 120.0, 80.0).unwrap();
        assert_eq!(pts.len(), 4, "{pts:?}");
        assert!((x - 60.0).abs() <= 1.5 && (y - 40.0).abs() <= 1.5 && (w - 98.0).abs() <= 2.5 && (h - 61.0).abs() <= 2.0, "{x} {y} {w} {h}");
        assert!(!super::boxy(&pts));
        let tri = |x: f32, y: f32| (20.0..=100.0).contains(&y) && (x - 120.0).abs() <= (100.0 - y) * 0.6;
        let img = image::RgbImage::from_fn(240, 160, |x, y| image::Rgb(if tri(x as f32, y as f32) { [200, 40, 50] } else { [150, 160, 170] }));
        let (_, _, _, _, pts) = super::wrap(&img, 60.0, 10.0, 120.0, 100.0).unwrap();
        assert_eq!(pts.len(), 3, "{pts:?}");
        // (a plain box: boxy, so it's kept as a box)
        let img = image::RgbImage::from_fn(200, 160, |x, y| image::Rgb(if (40..120).contains(&x) && (50..110).contains(&y) { [20, 20, 20] } else { [220, 220, 220] }));
        let (_, _, _, _, pts) = super::wrap(&img, 34.0, 46.0, 92.0, 70.0).unwrap();
        assert!(super::boxy(&pts), "{pts:?}");
        // (on a flat picture: nothing)
        assert!(super::wrap(&image::RgbImage::from_pixel(100, 100, image::Rgb([90, 90, 90])), 10.0, 10.0, 50.0, 40.0).is_none());
    }

    /// (by hand: HUD_IMG a screenshot, HUD_BOXES "x,y,w,h;..." boxes drawn on it; each one's wrap printed)
    #[test]
    #[ignore]
    fn wrap_boxes_on_a_screenshot() {
        let (Ok(f), Ok(b)) = (std::env::var("HUD_IMG"), std::env::var("HUD_BOXES")) else { return };
        let img = image::open(f).unwrap().to_rgb8();
        for bx in b.split(';') {
            let n: Vec<f32> = bx.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            println!("WRAP {bx} => {:?}", super::wrap(&img, n[0], n[1], n[2], n[3]));
        }
    }
}
