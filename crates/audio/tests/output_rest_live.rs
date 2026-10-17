// (live, on the real output device, silent: volume 0) The output stream rests after a few seconds of
// nothing to play, and starts again the moment there's something: playback goes on at once
// cargo test --release -p omnipulse-audio --test output_rest_live -- --ignored --nocapture

use omnipulse_audio::{engine::Engine, source::Tone};
use std::time::{Duration, Instant};

#[test]
#[ignore]
fn playback_starts_at_once_after_the_output_rests() {
    let e = Engine::new("").expect("an output device");
    e.set_volume(0.0);
    // (loaded and paused past the rest: the stream's stopped by now)
    e.load(Box::new(Tone::new(e.output_rate().max(44_100), 440.0, 0.2, 60.0)), 0.0);
    std::thread::sleep(Duration::from_secs(5));
    let before = e.position_ms();
    e.play();
    let t = Instant::now();
    // (playing again: its position moving within a quarter of a second)
    while e.position_ms() <= before + 50 && t.elapsed() < Duration::from_secs(3) { std::thread::sleep(Duration::from_millis(10)); }
    let took = t.elapsed();
    println!("position {before} -> {} ms, moving after {} ms", e.position_ms(), took.as_millis());
    assert!(took < Duration::from_millis(400), "it took {took:?} to start again");
    // (and paused again, rested again, then played: the same)
    e.pause();
    std::thread::sleep(Duration::from_secs(5));
    let before = e.position_ms();
    e.play();
    let t = Instant::now();
    while e.position_ms() <= before + 50 && t.elapsed() < Duration::from_secs(3) { std::thread::sleep(Duration::from_millis(10)); }
    println!("again: moving after {} ms", t.elapsed().as_millis());
    assert!(t.elapsed() < Duration::from_millis(400));
}
