// Seamless Mix in the desktop app: its planning is omnipulse_mix (crates/mix, shared with the Android
// app); here, the tests that need the app (renders, probes of real songs).

pub use omnipulse_mix::smart::*;

#[cfg(test)]
mod tests {
    /// The fallback fade the app uses (main.rs' SMART_FIXED_MS)
    const FIXED_MS: u64 = 6000;

    /// A 16-bit stereo WAV
    fn write_wav(path: &std::path::Path, out: &[f32], rate: u32) {
        let mut w = Vec::new();
        let data = (out.len() * 2) as u32;
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 2] { w.extend_from_slice(&v.to_le_bytes()); }
        for v in [rate, rate * 4] { w.extend_from_slice(&v.to_le_bytes()); }
        for v in [4u16, 16] { w.extend_from_slice(&v.to_le_bytes()); }
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data.to_le_bytes());
        for s in out { w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()); }
        std::fs::write(path, w).unwrap();
    }

    use super::*;

    /// Each one's onsets, a millisecond at a time: its lows' and its highs' (above 4 kHz: the hats) rises
    fn onsets_of(x: &[f32], rate: u32) -> Vec<f32> {
            let kl = 1.0 - (-2.0 * std::f32::consts::PI * 150.0 / rate as f32).exp();
            let kh = 1.0 - (-2.0 * std::f32::consts::PI * 4000.0 / rate as f32).exp();
            let (mut l1, mut l2, mut h1) = (0.0f32, 0.0f32, 0.0f32);
            let (lo, hi): (Vec<f32>, Vec<f32>) = x.chunks(2 * 44).map(|c| c.chunks(2).fold((0.0, 0.0), |(a, b), f| {
                let m = (f[0] + f[1]) * 0.5;
                l1 += kl * (m - l1); l2 += kl * (l1 - l2); h1 += kh * (m - h1);
                (a + l2 * l2, b + (m - h1) * (m - h1))
            })).unzip();
            let rise = |e: &[f32]| -> Vec<f32> { let l: Vec<f32> = e.iter().map(|v| (v + 1e-9).ln()).collect(); (0..l.len()).map(|i| if i >= 4 { (l[i] - l[i - 4]).max(0.0) } else { 0.0 }).collect() };
            let (a, b) = (rise(&lo), rise(&hi));
            a.iter().zip(&b).map(|(x, y)| x + y).collect()
    }

    /// Each one's kicks, as heard: its lows' envelope (a millisecond at a time, smoothed over 8), each sharp
    /// rise (half the strong ones' at least, 150 ms apart), timed where it's halfway up (ms)
    fn kicks_of(x: &[f32], rate: u32) -> Vec<f64> {
            let k = 1.0 - (-2.0 * std::f32::consts::PI * 150.0 / rate as f32).exp();
            let (mut z1, mut z2) = (0.0f32, 0.0f32);
            let raw: Vec<f32> = x.chunks(2 * 44).map(|c| c.chunks(2).map(|f| { z1 += k * ((f[0] + f[1]) * 0.5 - z1); z2 += k * (z1 - z2); z2 * z2 }).sum::<f32>()).collect();
            let n = raw.len();
            let e: Vec<f32> = (0..n).map(|i| { let (a, b) = (i.saturating_sub(4), (i + 4).min(n)); raw[a..b].iter().sum::<f32>() / (b - a) as f32 }).collect();
            let rise: Vec<f32> = (0..n).map(|i| if i >= 2 && i + 2 < n { e[i + 2] - e[i - 2] } else { 0.0 }).collect();
            let mut rs = rise.clone();
            rs.sort_by(|a, b| a.total_cmp(b));
            let line = rs[rs.len() * 995 / 1000] * 0.4;
            let mut out: Vec<f64> = Vec::new();
            let mut i = 2;
            while i + 30 < n {
                if rise[i] >= line && (i..i + 20).all(|j| rise[j] <= rise[i] || j == i) {
                    let base = e[i.saturating_sub(15)..=i].iter().copied().fold(f32::MAX, f32::min);
                    let top = e[i..i + 30].iter().copied().fold(0.0, f32::max);
                    let at = (i.saturating_sub(15)..i + 30).find(|&j| e[j] >= base + (top - base) * 0.5).unwrap_or(i);
                    out.push(at as f64 * 44.0 * 1000.0 / rate as f64);
                    i += 150;
                } else { i += 1; }
            }
            out
    }

    use omnipulse_core::rhythm::Measured;

    /// The offline renderer (step 3 of docs/dj-transitions.md): each pair as Smart Transitions would
    /// change it over, written to a WAV (from 12 s before it) with its plan beside it (a .json), to listen
    /// to and compare. Songs: SoundCloud ids, or paths to files.
    /// OMNIPULSE_SMART_PAIRS="111,222;333,C:/music/x.mp3" OMNIPULSE_SMART_OUT=renders cargo test --release -p omnipulse --bin OmniPulse-native smart_render -- --ignored --nocapture
    #[test]
    #[ignore]
    fn smart_render() {
        use crate::track::{Src, Track};
        let pairs = std::env::var("OMNIPULSE_SMART_PAIRS").expect("OMNIPULSE_SMART_PAIRS");
        let dir = std::path::PathBuf::from(std::env::var("OMNIPULSE_SMART_OUT").unwrap_or_else(|_| "smart-renders".into()));
        std::fs::create_dir_all(&dir).unwrap();
        let max_ms = std::env::var("OMNIPULSE_SMART_MAX").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(10) * 1000;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let kept: serde_json::Map<String, serde_json::Value> = omnipulse_core::data_dir().ok().and_then(|d| std::fs::read_to_string(d.join("state").join("yt-versions.json")).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let track = |s: &str| if let Some(id) = s.strip_prefix("yt:") {
            let v = kept.values().find(|v| v["id"].as_str() == Some(id)).cloned().unwrap_or_default();
            Track { src: Src::YouTube(id.to_string()), key: s.to_string(), title: v["title"].as_str().unwrap_or(id).to_string(), artist: v["artists"].as_str().unwrap_or("").to_string(), ms: v["duration_ms"].as_u64().unwrap_or(0), ..Default::default() }
        } else { match s.parse::<u64>() {
            Ok(id) => rt.block_on(omnipulse_core::soundcloud::api_get(format!("/tracks/{id}"))).ok().and_then(|v| Track::soundcloud(&v)).map(|mut t| { t.key = s.to_string(); t }).unwrap_or(Track { src: Src::SoundCloud(id), key: s.to_string(), ..Default::default() }),
            Err(_) => Track { src: Src::Local(s.to_string()), key: format!("local:{s}"), ..Default::default() },
        } };
        let what = |t: &Track| match &t.src { Src::SoundCloud(id) => ("soundcloud", id.to_string()), Src::Local(p) => ("local", p.clone()), Src::YouTube(id) => ("youtube", id.clone()), _ => unreachable!() };
        // (each measured whole, kept as the app keeps them; its loudness analysed: its silences, its end)
        let measure = |t: &Track| {
            let (kind, id) = what(t);
            let full = format!("full:{}", t.key);
            let m = omnipulse_core::rhythm::known(&full).filter(|m| !m.rhythm.bar_beat.is_empty()).or_else(|| {
                let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start(kind, &id, 1200.0)).ok()?;
                let heard = mono.len() as u64 * 1000 / rate.max(1) as u64;
                let m = Measured { from_ms: 0, to_ms: heard, rhythm: omnipulse_core::rhythm::analyse(&mono, rate)?, v: omnipulse_core::rhythm::VERSION };
                omnipulse_core::rhythm::keep(&full, m.clone());
                Some(m)
            });
            let an = rt.block_on(omnipulse_core::loudness::analyse(kind, &id)).unwrap_or_default();
            (m, an)
        };
        let rate = 44_100u32;
        let lead = 12_000u64;
        // (OMNIPULSE_SMART_VOICES=0: the voices not handed over, to compare)
        let voices = std::env::var("OMNIPULSE_SMART_VOICES").map_or(true, |v| v != "0");
        for (n, pair) in pairs.split(';').filter(|p| !p.trim().is_empty()).enumerate() {
            let (a_id, b_id) = pair.split_once(',').expect("a pair: A,B");
            let (ta, tb) = (track(a_id.trim()), track(b_id.trim()));
            let ((ma, ana), (mb, anb)) = (measure(&ta), measure(&tb));
            let shift_of = |id: &str| std::env::var("OMNIPULSE_SMART_SHIFTS").ok().and_then(|v| v.split(',').find_map(|x| x.rsplit_once(':').filter(|(k, _)| k.trim() == id).and_then(|(_, n)| n.trim().parse::<i32>().ok()))).unwrap_or(0);
            // (as the app has it: unless set by hand, its end's or its start's beat 1 where that's sure)
            let placed = |m: Measured, id: &str, end: bool| match shift_of(id) { 0 => local_downbeat(&m, end), n => shifted(&m, n) };
            let (ma, mb) = (ma.map(|m| placed(m, a_id.trim(), true)), mb.map(|m| placed(m, b_id.trim(), false)));
            // (OMNIPULSE_SMART_PHRASES="id:bars": where its phrases start, moved, as Track Preferences does)
            let phrase_of = |id: &str| std::env::var("OMNIPULSE_SMART_PHRASES").ok().and_then(|v| v.split(',').find_map(|x| x.rsplit_once(':').filter(|(k, _)| k.trim() == id).and_then(|(_, n)| n.trim().parse::<i32>().ok()))).unwrap_or(0);
            let with_phrase = |m: Option<Measured>, id: &str| m.map(|mut m| { m.rhythm.phrase_shift = phrase_of(id); m });
            let (ma, mb) = (with_phrase(ma, a_id.trim()), with_phrase(mb, b_id.trim()));
            // (their keys from outside, as the app has them: SongBPM's, in place of our readings)
            let ext = |t: &Track| rt.block_on(omnipulse_core::songbpm::lookup(t.artist.split(',').next().unwrap_or(""), &omnipulse_core::ranking::featured(&crate::versions::match_title(t)).1, t.ms)).ok().flatten();
            // (known only where ours and SongBPM's agree, as the app has it)
            let agrees = |m: &Option<Measured>, k: &Option<omnipulse_core::songbpm::SongKey>| matches!((m, k), (Some(m), Some(k)) if m.rhythm.key == omnipulse_core::rhythm::key_name(k.tonic, k.minor));
            let (ka, kb) = (ext(&ta), ext(&tb));
            let (ka_ok, kb_ok) = (agrees(&ma, &ka), agrees(&mb, &kb));
            println!("keys: ours {:?} / {:?}, SongBPM's {:?} / {:?}: known {ka_ok} / {kb_ok}", ma.as_ref().map(|m| &m.rhythm.key), mb.as_ref().map(|m| &m.rhythm.key), ka.as_ref().map(|k| omnipulse_core::rhythm::key_name(k.tonic, k.minor)), kb.as_ref().map(|k| omnipulse_core::rhythm::key_name(k.tonic, k.minor)));
            let open = |t: &Track| rt.block_on(crate::player::open_source(t, rt.handle().clone())).expect("it opens");
            let (mut sa, sb) = (open(&ta), open(&tb));
            let a_dur = sa.duration_ms().unwrap_or(0);
            let first = |an: &serde_json::Value| an["silences"].as_array().and_then(|s| s.first()).filter(|s| s[0].as_u64() == Some(0)).and_then(|s| s[1].as_u64()).unwrap_or(0);
            // (the voices handed over only where both sing, by their timed lyrics, as the app does; and the
            // blend placed so the one playing's singer isn't cut off: fetched before it's planned)
            let lyrics = |t: &Track, m: &Option<Measured>| rt.block_on(crate::player::mix_lyrics(t, m.as_ref().map_or(0, |m| m.to_ms / 1000), &serde_json::Value::Null));
            let (la, lb) = (lyrics(&ta, &ma), lyrics(&tb, &mb));
            println!("timed lyrics: {} lines / {} lines", la.len(), lb.len());
            let a = Side { dur_ms: a_dur, audible_ms: ana["audible_ms"].as_u64().unwrap_or(0), first_ms: 0, rhythm: ma.as_ref(), stated_bpm: None, key_known: ka_ok, lines: &la };
            let b = Side { dur_ms: anb["duration_ms"].as_u64().unwrap_or(0), audible_ms: anb["audible_ms"].as_u64().unwrap_or(0), first_ms: first(&anb), rhythm: mb.as_ref(), stated_bpm: None, key_known: kb_ok, lines: &lb };
            let db = |t: &Track, an: &serde_json::Value| crate::sound::loud_db(&serde_json::json!({}), t, an["lufs"].as_f64(), false);
            let dbs = (db(&ta, &ana), db(&tb, &anb));
            let blocked = blend_blocked(&a, &b);
            let forced = std::env::var_os("OMNIPULSE_SMART_FORCE").is_some();
            let mut plan = blend_as(&a, &b, 0, forced).or_else(|| std::env::var_os("OMNIPULSE_SMART_ROLL").and_then(|_| roll(&a, &b, 0)));
            // (OMNIPULSE_SMART_NUDGE: the next one started this many of its beats later (or, below 0, earlier), to listen for which is right)
            let nudge = std::env::var("OMNIPULSE_SMART_NUDGE").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            if let (Some(p), Some(m)) = (plan.as_mut(), mb.as_ref()) { p.start_ms = (p.start_ms as f64 + nudge * m.rhythm.period_ms).max(0.0).round() as u64; }
            let both_sing = |a_from: u64, a_to: u64, b_from: u64, b_to: u64| sings(&la, a_from, a_to) && sings(&lb, b_from, b_to);
            let (out, info) = match &plan {
                Some(p) if p.style == Some(ROLL) => {
                    sa.seek(p.at_ms.saturating_sub(lead)).unwrap();
                    let mut sb = sb;
                    sb.seek(p.start_ms).unwrap();
                    let out = omnipulse_audio::engine::render_mix(sa, sb, dbs, rate, lead as f32 / 1000.0, p.ms, Some(handoff(p, false)), (lead + p.ms) as f32 / 1000.0 + 15.0).unwrap();
                    (out, serde_json::json!({ "kind": "roll", "plan": p, "notOnTheBeat": blocked }))
                }
                Some(p) => {
                    sa.seek(p.at_ms.saturating_sub(lead)).unwrap();
                    let b_tempo = omnipulse_audio::Tempo::new(p.rate);
                    let mut sb = omnipulse_audio::Stretched::shifted(sb, b_tempo.clone(), p.semitones).unwrap();
                    let b_pitch = sb.pitch();
                    omnipulse_audio::Source::seek(&mut sb, p.start_ms).unwrap();
                    let (target, a_tempo, a_pitch) = (p.out_tempo, omnipulse_audio::Tempo::new(1.0), omnipulse_audio::Tempo::new(1.0));
                    let share = p.ms as f64 / p.ramp_ms.max(1) as f64;
                    // (and a key ride, as the app's: both together, the next one in its own key by the swap)
                    let (s, swap_share) = (p.semitones as f64, p.ms as f64 / p.swap_ms.max(1) as f64);
                    let ramp = { let (a_tempo, a_pitch) = (a_tempo.clone(), a_pitch.clone()); move |x: f64| {
                        a_tempo.set(omnipulse_mix::plan::out_ramp(target, x * share)); b_tempo.set(omnipulse_mix::plan::in_ramp(target, x * share));
                        if s != 0.0 { let q = (x * swap_share).min(1.0); let e = q * q * (3.0 - 2.0 * q); b_pitch.set(2f64.powf(s * (1.0 - e) / 12.0)); a_pitch.set(2f64.powf(-s * e / 12.0)); }
                    } };
                    // (OMNIPULSE_SMART_SOLO=a or b: that one alone, as it's heard through the blend (ridden as it
                    // is, never faded), to time its kicks against the other's: blend_ride_probe)
                    let solo = std::env::var("OMNIPULSE_SMART_SOLO").ok();
                    let held = |b_in: bool| { let far = 1_000_000_000; let r = omnipulse_audio::Ramp { in_at: if b_in { 0 } else { far }, in_len: 1, out_at: far, out_len: 1 };
                        omnipulse_audio::Handoff { low: r, mid: r, high: r, out_sweep: None, brake: None, spin: None, roll: None, out_voice: None, in_voice: None } };
                    let (dbs, h) = match solo.as_deref() {
                        Some("a") => ((dbs.0, -300.0), held(false)),
                        Some("b") => ((-300.0, dbs.1), held(true)),
                        _ => (dbs, handoff(p, voices && both_sing(p.at_ms, p.at_ms + p.ms, p.start_ms, p.start_ms + (p.ms as f64 * p.rate) as u64))),
                    };
                    let out = omnipulse_audio::engine::render_mix_keyed(sa, Box::new(sb), dbs, rate, lead as f32 / 1000.0, p.ms, Some(h), (target != 1.0 || s != 0.0).then_some((&a_tempo, &ramp as &dyn Fn(f64))), (s != 0.0).then_some(&a_pitch), (lead + p.ms) as f32 / 1000.0 + 15.0).unwrap();
                    (out, serde_json::json!({ "kind": "smart blend", "plan": p, "nudgeBeats": nudge, "voicesHanded": voices && both_sing(p.at_ms, p.at_ms + p.ms, p.start_ms, p.start_ms + (p.ms as f64 * p.rate) as u64), "lyricLines": [la.len(), lb.len()] }))
                }
                None => {
                    let f = fade(&a, &b, max_ms, FIXED_MS);
                    sa.seek(f.out_from_ms.saturating_sub(lead)).unwrap();
                    let mut sb = sb;
                    if f.in_from_ms > 0 { sb.seek(f.in_from_ms).unwrap(); }
                    let out = omnipulse_audio::engine::render_mix(sa, sb, dbs, rate, lead as f32 / 1000.0, f.len_ms(), fade_handoff(f.len_ms(), voices && both_sing(f.out_from_ms, f.out_to_ms, f.in_from_ms, f.in_from_ms + f.len_ms())), (lead + f.len_ms()) as f32 / 1000.0 + 15.0).unwrap();
                    (out, serde_json::json!({ "kind": "smart fade", "outFromMs": f.out_from_ms, "outToMs": f.out_to_ms, "inFromMs": f.in_from_ms, "ms": f.len_ms(), "notOnTheBeat": blocked.unwrap_or("no phrase to blend on"),
                        "outroFromMs": ma.as_ref().and_then(outro_from), "introToMs": mb.as_ref().and_then(intro_to) }))
                }
            };
            let side = |m: &Option<Measured>| m.as_ref().map(|m| serde_json::json!({ "bpm": m.rhythm.bpm, "confidence": m.rhythm.tempo_confidence, "camelot": m.rhythm.camelot, "keyConfidence": m.rhythm.key_confidence, "downbeatConfidence": m.rhythm.downbeat_confidence, "groups": groups(m) }));
            let info = serde_json::json!({ "a": a_id, "b": b_id, "titles": [format!("{} - {}", ta.artist, ta.title), format!("{} - {}", tb.artist, tb.title)], "aMeasured": side(&ma), "bMeasured": side(&mb), "dbs": [dbs.0, dbs.1], "startsAt": "12 s in", "transition": info });
            let base = dir.join(format!("{:02}", n + 1));
            write_wav(&base.with_extension("wav"), &out, rate);
            std::fs::write(base.with_extension("json"), serde_json::to_string_pretty(&info).unwrap()).unwrap();
            println!("{}: {}", base.display(), info["transition"]);
        }
    }

    /// Where a seek really lands, as heard: each song (OMNIPULSE_SEEK_IDS, as smart_render takes them) played
    /// from its start, against it opened and sought to 60 s, as is and through the stretcher (as the next
    /// one in a blend is, at its own tempo): the lag (ms, +: the sought one late) where they match
    #[test]
    #[ignore]
    fn seek_offsets() {
        use crate::track::{Src, Track};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let rate = 44_100u32;
        for id in std::env::var("OMNIPULSE_SEEK_IDS").unwrap().split(',') {
            let t = match id.strip_prefix("yt:") { Some(v) => Track { src: Src::YouTube(v.into()), key: id.into(), ..Default::default() }, None => Track { src: Src::SoundCloud(id.parse().unwrap()), key: id.into(), ..Default::default() } };
            let open = || rt.block_on(crate::player::open_source(&t, rt.handle().clone())).expect("it opens");
            let mono = |x: Vec<f32>| -> Vec<f32> { x.chunks(2).map(|f| (f[0] + f[1]) * 0.5).collect() };
            let whole = mono(omnipulse_audio::engine::render(open(), rate, &[0.0; 10], 0.0, 72.0).unwrap());
            let mut sa = open();
            let landed = sa.seek(60_000).unwrap();
            let plain = mono(omnipulse_audio::engine::render(sa, rate, &[0.0; 10], 0.0, 8.0).unwrap());
            let mut sb = omnipulse_audio::Stretched::shifted(open(), omnipulse_audio::Tempo::new(1.0), 0).unwrap();
            omnipulse_audio::Source::seek(&mut sb, 60_000).unwrap();
            let stretched = mono(omnipulse_audio::engine::render(Box::new(sb), rate, &[0.0; 10], 0.0, 8.0).unwrap());
            // (the lag where the sought one's first 6 s match the whole one's from 60 s, in 0.1 ms steps of 1 frame)
            let lag = |x: &[f32]| -> (f64, f32) {
                let at = 60 * rate as i64;
                let n = 6 * rate as usize;
                let corr = |l: i64| (0..n).step_by(2).map(|i| x[i] * whole[(at + i as i64 - l) as usize]).sum::<f32>();
                let best = (-(rate as i64) / 5..=(rate as i64) / 5).map(|l| (l, corr(l))).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
                let norm = ((0..n).step_by(2).map(|i| x[i] * x[i]).sum::<f32>() * (0..n).step_by(2).map(|i| whole[at as usize + i - best.0 as usize] .powi(2)).sum::<f32>()).sqrt().max(1e-9);
                (best.0 as f64 * 1000.0 / rate as f64, best.1 / norm)
            };
            let (p, s) = (lag(&plain), lag(&stretched));
            println!("{id}: seek said {landed} ms; as heard: plain {:+.1} ms (match {:.2}), stretched {:+.1} ms (match {:.2})", p.0, p.1, s.0, s.1);
        }
    }

    /// How a blend's beats line up through its tempo ride, as heard: the two solo renders of it (smart_render
    /// with OMNIPULSE_SMART_SOLO=a, then =b, into OMNIPULSE_PROBE_A and OMNIPULSE_PROBE_B), their kicks over
    /// the blend (from 12 s in) compared: each of A's with B's nearest, by when (+: B late)
    #[test]
    #[ignore]
    fn blend_ride_probe() {
        let read = |v: &str| -> Vec<f32> { let b = std::fs::read(std::env::var(v).unwrap()).unwrap(); b[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect() };
        let (a, b) = (read("OMNIPULSE_PROBE_A"), read("OMNIPULSE_PROBE_B"));
        let (ka, kb) = (kicks_of(&a, 44_100), kicks_of(&b, 44_100));
        let mut line = Vec::new();
        for t in ka.iter().filter(|t| **t >= 12_000.0) {
            let Some(d) = kb.iter().map(|x| x - t).min_by(|x, y| x.abs().total_cmp(&y.abs())) else { continue };
            line.push(format!("{:.1}s {:+.0}", t / 1000.0, d));
        }
        println!("kicks A {} B {}; B against A by A's kick: {}", ka.len(), kb.len(), line.join(", "));
        // (and their onsets matched 4 s at a time, every 2 s: the lag (ms, within 300 either way) where they
        // match best, and how much better than at no lag)
        let (oa, ob) = (onsets_of(&a, 44_100), onsets_of(&b, 44_100));
        let n = oa.len().min(ob.len());
        let mut w = Vec::new();
        let mut t = 12_000usize;
        while t + 4_000 + 300 < n {
            let corr = |l: i64| (t..t + 4_000).map(|i| oa[i] * ob[(i as i64 + l) as usize]).sum::<f32>();
            let best = (-300..=300).map(|l| (l, corr(l))).max_by(|x, y| x.1.total_cmp(&y.1)).unwrap();
            w.push(format!("{:.0}s {:+} ({:.2}x)", t as f64 / 1000.0, best.0, best.1 / corr(0).max(1e-9)));
            t += 2_000;
        }
        println!("windows: {}", w.join(", "));
    }

    /// How well a blend's beats line up as heard: each side rendered alone over the blend (the next one
    /// stretched and started as the plan has it), their kicks (below 150 Hz, where they rise) compared a
    /// bar at a time: the lag (ms) at which they match best, B against A (+: B late). A steady lag is an
    /// offset; one that grows, a tempo off.
    /// OMNIPULSE_SMART_PAIRS="A,B;..." cargo test --release -p omnipulse --bin OmniPulse-native blend_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn blend_probe() {
        use crate::track::{Src, Track};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let rate = 44_100u32;
        for pair in std::env::var("OMNIPULSE_SMART_PAIRS").unwrap().split(';') {
            let (a_id, b_id) = pair.split_once(',').unwrap();
            let t = |id: &str| Track { src: Src::SoundCloud(id.parse().unwrap()), key: id.to_string(), ..Default::default() };
            let (ta, tb) = (t(a_id), t(b_id));
            let full = |t: &Track| omnipulse_core::rhythm::known(&format!("full:{}", t.key)).expect("measured whole (run smart_render first)");
            let (ma, mb) = (full(&ta), full(&tb));
            let an = |id: &str| rt.block_on(omnipulse_core::loudness::analyse("soundcloud", id)).unwrap_or_default();
            let (ana, anb) = (an(a_id), an(b_id));
            let a = Side { dur_ms: ana["duration_ms"].as_u64().unwrap_or(0), audible_ms: ana["audible_ms"].as_u64().unwrap_or(0), rhythm: Some(&ma), ..Default::default() };
            let b = Side { dur_ms: anb["duration_ms"].as_u64().unwrap_or(0), audible_ms: anb["audible_ms"].as_u64().unwrap_or(0), rhythm: Some(&mb), ..Default::default() };
            let Some(p) = blend(&a, &b, 0) else { println!("{pair}: no blend ({:?})", blend_blocked(&a, &b)); continue };
            let open = |t: &Track| rt.block_on(crate::player::open_source(t, rt.handle().clone())).expect("it opens");
            let secs = p.ms as f32 / 1000.0;
            let mut sa = open(&ta);
            sa.seek(p.at_ms).unwrap();
            let pa = omnipulse_audio::engine::render(sa, rate, &[0.0; 10], 0.0, secs).unwrap();
            // (OMNIPULSE_PROBE_CONTROL: B is A itself, a bar on: its kicks should match at 0)
            let control = std::env::var_os("OMNIPULSE_PROBE_CONTROL").is_some();
            let (tb, start, r) = if control { (ta.clone(), bar_ms(&ma, (ma.rhythm.beats.iter().position(|x| ma.from_ms as f64 + x >= p.at_ms as f64).unwrap_or(0).saturating_sub(ma.rhythm.downbeat)) / 4 + 1).unwrap().round() as u64, 1.0) } else { (tb.clone(), p.start_ms, p.rate) };
            let mut sb = omnipulse_audio::Stretched::shifted(open(&tb), omnipulse_audio::Tempo::new(r), 0).unwrap();
            omnipulse_audio::Source::seek(&mut sb, start).unwrap();
            let pb = omnipulse_audio::engine::render(Box::new(sb), rate, &[0.0; 10], 0.0, secs).unwrap();
            // (each one's kicks, as heard: its lows' envelope (a millisecond at a time, smoothed over 8), each
            // sharp rise (half the strong ones' at least, 150 ms apart), timed where it's halfway up)
            let kicks = |x: &[f32]| kicks_of(x, rate);
            // (over the whole blend at once: each one's onsets, its lows' and its highs' rises (above 4 kHz:
            // the hats), a millisecond at a time, and the lag where they match best)
            let onsets = |x: &[f32]| onsets_of(x, rate);
            let (oa, ob) = (onsets(&pa), onsets(&pb));
            let n = oa.len().min(ob.len());
            let corr = |l: i64| (100..n - 100).map(|i| oa[i] * ob[(i as i64 + l) as usize]).sum::<f32>();
            let curve: Vec<(i64, f32)> = (-80..=80).map(|l| (l, corr(l))).collect();
            let best = curve.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            let zero = curve.iter().find(|c| c.0 == 0).unwrap().1;
            println!("{pair}: whole blend, best lag {} ms (its match {:.0}, at 0 ms {:.0})", best.0, best.1, zero);
            let (ka, kb) = (kicks(&pa), kicks(&pb));
            let d: Vec<i64> = ka.iter().filter_map(|a| kb.iter().map(|b| b - a).min_by(|x, y| x.abs().total_cmp(&y.abs())).filter(|x| x.abs() < 120.0).map(|x| x.round() as i64)).collect();
            let mut sorted = d.clone();
            sorted.sort();
            println!("{pair}: rate {:.4}, kicks A {} B {}, B against A (ms, +: B late), median {:?}: {d:?}", p.rate, ka.len(), kb.len(), sorted.get(sorted.len() / 2));
        }
    }

    /// Each song measured again (as the app measures it whole), its downbeat's clues shown
    /// OMNIPULSE_RHYTHM_DEBUG=1 OMNIPULSE_PROBE_IDS=1,2 cargo test --release -p omnipulse --bin OmniPulse-native rhythm_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn rhythm_probe() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for id in std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',') {
            let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", id, 1200.0)).unwrap();
            eprintln!("{id}:");
            let r = omnipulse_core::rhythm::analyse(&mono, rate).unwrap();
            eprintln!("  {} BPM, downbeat {}, first beats {:?}", r.bpm, r.downbeat, &r.beats[..4]);
        }
    }

    /// What measuring a song costs: wall time, and the CPU time all its threads took (Windows)
    /// OMNIPULSE_PROBE_IDS=1 cargo test --release -p omnipulse --bin OmniPulse-native rhythm_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn rhythm_cost() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cpu = || -> f64 {
            #[cfg(windows)]
            unsafe {
                #[repr(C)] #[derive(Default, Clone, Copy)] struct Ft { lo: u32, hi: u32 }
                #[link(name = "kernel32")] extern "system" { fn GetCurrentProcess() -> isize; fn GetProcessTimes(h: isize, c: *mut Ft, e: *mut Ft, k: *mut Ft, u: *mut Ft) -> i32; }
                let (mut c, mut e, mut k, mut u) = (Ft::default(), Ft::default(), Ft::default(), Ft::default());
                GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
                let t = |f: Ft| ((f.hi as u64) << 32 | f.lo as u64) as f64 / 1e7;
                return t(k) + t(u);
            }
            #[allow(unreachable_code)] 0.0
        };
        for id in std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',') {
            let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", id, 1200.0)).unwrap();
            let secs = mono.len() as f64 / rate as f64;
            // (once to load the model, then measured)
            let _ = omnipulse_core::rhythm::analyse(&mono[..(rate as usize * 10).min(mono.len())], rate);
            let (c0, t0) = (cpu(), std::time::Instant::now());
            let _ = omnipulse_core::rhythm::analyse(&mono, rate);
            let (c1, t1) = (cpu(), t0.elapsed().as_secs_f64());
            eprintln!("{id}: {secs:.0} s of song: {t1:.2} s to measure, {:.2} s of CPU ({:.1} cores while it ran), {} cores here", c1 - c0, (c1 - c0) / t1, std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));
        }
    }

    /// A SoundCloud track's own details (its bpm, genre, description), as its API gives them
    #[test]
    #[ignore]
    fn sc_details() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for id in std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',') {
            let v = rt.block_on(omnipulse_core::soundcloud::api_get(format!("/tracks/{id}"))).unwrap_or_default();
            eprintln!("{id}: {} | bpm {} | genre {} | tags {} | {}", v["title"], v["bpm"], v["genre"], v["tag_list"], v["description"].as_str().unwrap_or("").chars().take(300).collect::<String>().replace(['\n', '\r'], " "));
        }
    }

    /// Memory as songs are measured one after another (Windows: the process's working set and private
    /// bytes after each): OMNIPULSE_PROBE_IDS=1,2,3...
    #[test]
    #[ignore]
    fn rhythm_memory() {
        #[cfg(windows)]
        fn mem() -> (f64, f64) {
            #[repr(C)] #[derive(Default)] struct Pmc { cb: u32, faults: u32, peak_ws: usize, ws: usize, qpp: usize, qp: usize, qnpp: usize, qnp: usize, pagefile: usize, peak_pagefile: usize, private: usize }
            #[link(name = "kernel32")] extern "system" { fn GetCurrentProcess() -> isize; fn K32GetProcessMemoryInfo(h: isize, p: *mut Pmc, cb: u32) -> i32; }
            let mut p = Pmc { cb: std::mem::size_of::<Pmc>() as u32, ..Default::default() };
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb); }
            (p.ws as f64 / 1e6, p.private as f64 / 1e6)
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ids: Vec<String> = std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',').map(String::from).collect();
        eprintln!("start: {:?} MB (working set, private)", mem());
        for round in 0..2 {
            for id in &ids {
                let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", id, 1200.0)).unwrap();
                let secs = mono.len() / rate as usize;
                let _ = omnipulse_core::rhythm::analyse(&mono, rate);
                drop(mono);
                eprintln!("round {round}, {id} ({secs} s): {:?} MB", mem());
            }
        }
    }
}
