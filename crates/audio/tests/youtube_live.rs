// YouTube through the engine, for real (uses the yt-dlp the app set up; plays nothing aloud):
//   cargo test -p omnipulse-audio --test youtube_live -- --ignored --nocapture

#[test]
#[ignore]
fn youtube_m4a_plays_and_seeks() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let url = rt.block_on(omnipulse_core::yt_stream::resolve_video("dQw4w9WgXcQ")).expect("yt-dlp resolves it");
    println!("stream: {}...", &url[..url.len().min(60)]);
    let t = std::time::Instant::now();
    let src = omnipulse_audio::open_url(&url, "m4a").expect("opens");
    let (rate, dur) = (omnipulse_audio::Source::rate(&src), omnipulse_audio::Source::duration_ms(&src));
    println!("opened in {} ms: {rate} Hz, {:?} ms long", t.elapsed().as_millis(), dur);
    let pcm = omnipulse_audio::render(Box::new(src), 48000, &[0.0; 10], 0.0, 30.0).unwrap();
    let (rms, peak) = omnipulse_audio::dsp::level(&pcm);
    println!("30 s rendered in {} ms total: {rms:.1} dB rms, {peak:.1} dB peak", t.elapsed().as_millis());
    assert!(pcm.len() >= 48000 * 2 * 29 && rms > -40.0);
    // a seek fetches from there, not the whole file
    let mut src = omnipulse_audio::open_url(&url, "m4a").unwrap();
    let t = std::time::Instant::now();
    let at = omnipulse_audio::Source::seek(&mut src, 90_000).unwrap();
    let mut piece = Vec::new();
    assert!(omnipulse_audio::Source::read(&mut src, &mut piece).unwrap());
    println!("seek to 90 s landed at {at} ms in {} ms", t.elapsed().as_millis());
    assert!((at as i64 - 90_000).abs() < 1000);
}
