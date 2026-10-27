// Every scene made and drawn on this PC's GPU, as the app would (a beat of made-up music under it): each
// frame's time, and its last frame kept as a picture when OMNIPULSE_VIS_OUT names a folder. No GPU: said, not failed.

use omnipulse_vis::{audio::{Analyser, WINDOW}, scenes, Renderer};

#[test]
fn every_scene_draws() {
    let mut r = match Renderer::new() { Ok(r) => r, Err(e) => { eprintln!("skipped: {e}"); return; } };
    eprintln!("GPU: {}", r.adapter());
    let rate = 48000u32;
    // (a kick every half second, hats between, a riser from 2.5 s, the drop at 3.5 s)
    let secs = 4.2f32;
    let n = (secs * rate as f32) as usize;
    let mut seed = 7u32;
    let (mut l, mut rr) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut prev = 0.0f32;
    for i in 0..n {
        let t = i as f32 / rate as f32;
        seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
        let w = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
        let hi = w - prev; prev = w;
        let riser = (2.5..3.5).contains(&t);
        let k = t % 0.5;
        let mut v = if riser { 0.0 } else { 0.8 * (std::f32::consts::TAU * 55.0 * k).sin() * (-k * 18.0).exp() };
        v += if riser { 0.3 * hi * (t - 2.5) } else { 0.1 * hi * (-((t + 0.25) % 0.5) * 60.0).exp() };
        let tone = 0.3 * (std::f32::consts::TAU * 220.0 * t).sin();
        l.push(v + tone);
        rr.push(v + 0.3 * (std::f32::consts::TAU * 220.0 * t + 1.2).sin());
    }
    let out = std::env::var_os("OMNIPULSE_VIS_OUT").map(std::path::PathBuf::from);
    let (w, h) = (960u32, 540u32);
    // (a cover, for the scenes that use one: OMNIPULSE_VIS_ART's picture, or a made-up one)
    let art = std::env::var_os("OMNIPULSE_VIS_ART").and_then(|p| image::ImageReader::open(p).ok()?.with_guessed_format().ok()?.decode().ok()).map(|i| i.to_rgba8()).unwrap_or_else(|| {
        image::RgbaImage::from_fn(300, 300, |x, y| { let d = ((x as f32 - 150.0).hypot(y as f32 - 150.0) / 15.0) as u32; image::Rgba(if d % 2 == 0 { [230, 90, 60, 255] } else { [30, 40, 90, 255] }) })
    });
    r.set_art(Some((art.as_raw(), art.width(), art.height())));
    for scene in scenes::all() {
        r.set_scene(scene).unwrap_or_else(|e| panic!("{}: {e}", scene.id));
        let mut a = Analyser::new(rate);
        let fps = 60.0;
        let mut frames = 0;
        let mut spent = std::time::Duration::ZERO;
        let mut last = Vec::new();
        let mut at = WINDOW;
        while at <= n {
            let audio = a.analyse(&l[at - WINDOW..at], &rr[at - WINDOW..at], 1.0 / fps).clone();
            let t0 = std::time::Instant::now();
            last = r.render(w, h, at as f32 / rate as f32, &audio).unwrap_or_else(|e| panic!("{}: {e}", scene.id));
            if frames > 5 { spent += t0.elapsed(); }
            // (a calm moment too, before the build: at 2 s)
            if frames == 120 { if let Some(dir) = &out { let _ = std::fs::create_dir_all(dir); image::save_buffer(dir.join(format!("{}-calm.png", scene.id)), &last, w, h, image::ColorType::Rgba8).unwrap(); } }
            frames += 1;
            at += (rate as f32 / fps) as usize;
        }
        let ms = spent.as_secs_f64() * 1000.0 / (frames - 6).max(1) as f64;
        // (something drawn: not all one colour)
        let lit = last.chunks(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 60).count();
        eprintln!("{}: {frames} frames at {w}x{h}, {ms:.2} ms each (with reading it back), {:.1}% lit", scene.id, lit as f64 * 100.0 / (w * h) as f64);
        assert!(lit > (w * h / 200) as usize, "{} drew next to nothing", scene.id);
        if let Some(dir) = &out {
            let _ = std::fs::create_dir_all(dir);
            image::save_buffer(dir.join(format!("{}.png", scene.id)), &last, w, h, image::ColorType::Rgba8).unwrap();
        }
    }
}

// The scenes as OBS's /vis page reads them, and a frame of made-up sound: written to OMNIPULSE_VIS_WEBGL's folder
// (scenes.json, frame.json), to try the page against; the JSON checked for its shape either way
#[test]
fn webgl_scenes_and_a_frame() {
    let json = omnipulse_vis::webgl_json();
    assert!(json.starts_with("{\"prelude\":") && json.contains("\"smoke\":") && json.contains("\"phosphor\":"), "{}", &json[..200.min(json.len())]);
    let mut a = omnipulse_vis::audio::Analyser::new(48000);
    let l: Vec<f32> = (0..2048).map(|i| (i as f32 * 0.05).sin() * 0.6).collect();
    let audio = a.analyse(&l, &l, 1.0 / 30.0).clone();
    let frame = omnipulse_vis::frame_json(&audio, 1);
    assert!(frame.contains("\"tex\":\""), "{frame}");
    if let Some(dir) = std::env::var_os("OMNIPULSE_VIS_WEBGL").map(std::path::PathBuf::from) {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("scenes.json"), &json).unwrap();
        std::fs::write(dir.join("frame.json"), &frame).unwrap();
    }
}
