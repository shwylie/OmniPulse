// Tries YouTube Music from the command line:  cargo run -p omnipulse-core --example yt -- <search words>
use omnipulse_core::{youtube, yt_stream};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let q = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    println!("installed: {}", yt_stream::status().await);
    let found = youtube::search(&q).await.expect("search failed");
    println!("search \"{q}\": {} songs", found.len());
    for t in found.iter().take(5) {
        println!("  {} | {} | {} | {} ms | {}", t.id, t.title, t.artists, t.duration_ms, t.album);
    }
    let Some(first) = found.first() else { return };
    let radio = youtube::radio(&first.id).await.expect("radio failed");
    println!("radio from {}: {} songs", first.title, radio.len());
    for t in radio.iter().skip(1).take(5) {
        println!("  {} | {} | {} | {} ms", t.id, t.title, t.artists, t.duration_ms);
    }
    match youtube::stream_url(&first.id, false).await {
        Ok(u) => println!("stream: {}...", &u[..u.len().min(90)]),
        Err(e) => println!("stream: {e}"),
    }
}
