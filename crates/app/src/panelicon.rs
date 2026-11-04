// Channel panels' icons: the ones to pick from (the same keys and shapes as the panel page's, widget/panels.html),
// the one a title picks when none's chosen, and a picture of yours made into an icon (its background cut out, the
// rest one colour, for the page to show in the panels' colours as it does the others).
//
// The shapes are Material Design icons (Apache 2.0) on a 24 x 24 grid, and a few of OmniPulse's own.

/// The icons to pick from: (key, name, shape)
pub const ICONS: [(&str, &str, &str); 39] = [
    ("user", "Person", "M12 12a5 5 0 1 0 0-10 5 5 0 0 0 0 10zm0 2c-4.4 0-8 2.2-8 5v3h16v-3c0-2.8-3.6-5-8-5z"),
    ("people", "People", "M16 11c1.66 0 2.99-1.34 2.99-3S17.66 5 16 5c-1.66 0-3 1.34-3 3s1.34 3 3 3zm-8 0c1.66 0 2.99-1.34 2.99-3S9.66 5 8 5C6.34 5 5 6.34 5 8s1.34 3 3 3zm0 2c-2.33 0-7 1.17-7 3.5V19h14v-2.5c0-2.33-4.67-3.5-7-3.5zm8 0c-.29 0-.62.02-.97.05 1.16.84 1.97 1.97 1.97 3.45V19h6v-2.5c0-2.33-4.67-3.5-7-3.5z"),
    ("calendar", "Calendar", "M7 2v2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2h-2V2h-2v2H9V2zm-2 8h14v10H5z"),
    ("clock", "Clock", "M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z"),
    ("shield", "Shield", "M12 2l8 3v6c0 5-3.4 9.4-8 11-4.6-1.6-8-6-8-11V5z"),
    ("info", "Info", "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z"),
    ("question", "Question", "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm1 15h-2v-2h2zm2.1-7.7l-.9.9c-.7.7-1.2 1.3-1.2 2.8h-2v-.5c0-1.1.5-2.1 1.2-2.8l1.2-1.3A2 2 0 1 0 10 10H8a4 4 0 1 1 7.1 2.3z"),
    ("link", "Link", "M10.6 13.4a1 1 0 0 1 0-1.4l3.5-3.5a3 3 0 1 1 4.2 4.2l-2 2-1.4-1.4 2-2a1 1 0 0 0-1.4-1.4L12 13.4a1 1 0 0 1-1.4 0zM13.4 10.6a1 1 0 0 1 0 1.4l-3.5 3.5a3 3 0 1 1-4.2-4.2l2-2 1.4 1.4-2 2a1 1 0 0 0 1.4 1.4L12 10.6a1 1 0 0 1 1.4 0z"),
    ("chat", "Chat", "M4 4h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H8l-4 4V6a2 2 0 0 1 2-2z"),
    ("mail", "Mail", "M20 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2zm0 4l-8 5-8-5V6l8 5 8-5v2z"),
    ("bell", "Bell", "M12 22c1.1 0 2-.9 2-2h-4c0 1.1.89 2 2 2zm6-6v-5c0-3.07-1.64-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.63 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z"),
    ("heart", "Heart", "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54z"),
    ("money", "Money", "M11.8 10.9c-2.27-.59-3-1.2-3-2.15 0-1.09 1.01-1.85 2.7-1.85 1.78 0 2.44.85 2.5 2.1h2.21c-.07-1.72-1.12-3.3-3.21-3.81V3h-3v2.16c-1.94.42-3.5 1.68-3.5 3.61 0 2.31 1.91 3.46 4.7 4.13 2.5.6 3 1.48 3 2.41 0 .69-.49 1.79-2.7 1.79-2.06 0-2.87-.92-2.98-2.1h-2.2c.12 2.19 1.76 3.42 3.68 3.83V21h3v-2.15c1.95-.37 3.5-1.5 3.5-3.55 0-2.84-2.43-3.81-4.7-4.4z"),
    ("gift", "Gift", "M20 6h-2.18c.11-.31.18-.65.18-1 0-1.66-1.34-3-3-3-1.05 0-1.96.54-2.5 1.35l-.5.67-.5-.68C10.96 2.54 10.05 2 9 2 7.34 2 6 3.34 6 5c0 .35.07.69.18 1H4c-1.11 0-1.99.89-1.99 2L2 19c0 1.11.89 2 2 2h16c1.11 0 2-.89 2-2V8c0-1.11-.89-2-2-2zm-5-2c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zM9 4c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm11 15H4v-2h16v2zm0-5H4V8h5.08L7 10.83 8.62 12 11 8.76l1-1.36 1 1.36L15.38 12 17 10.83 14.92 8H20v6z"),
    ("bag", "Shop", "M7 7V6a5 5 0 0 1 10 0v1h3l-1 15H5L4 7zm2 0h6V6a3 3 0 0 0-6 0z"),
    ("cart", "Cart", "M7 18c-1.1 0-1.99.9-1.99 2S5.9 22 7 22s2-.9 2-2-.9-2-2-2zM1 2v2h2l3.6 7.59-1.35 2.45c-.16.28-.25.61-.25.96 0 1.1.9 2 2 2h12v-2H7.42c-.14 0-.25-.11-.25-.25l.03-.12.9-1.63h7.45c.75 0 1.41-.41 1.75-1.03l3.58-6.49A1.003 1.003 0 0 0 20 4H5.21l-.94-2H1zm16 16c-1.1 0-1.99.9-1.99 2s.89 2 1.99 2 2-.9 2-2-.9-2-2-2z"),
    ("pc", "Computer", "M3 4h18v12H3zm6 14h6v2H9z"),
    ("keyboard", "Keyboard", "M20 5H4c-1.1 0-1.99.9-1.99 2L2 17c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm-9 3h2v2h-2V8zm0 3h2v2h-2v-2zM8 8h2v2H8V8zm0 3h2v2H8v-2zm-1 2H5v-2h2v2zm0-3H5V8h2v2zm9 7H8v-2h8v2zm0-4h-2v-2h2v2zm0-3h-2V8h2v2zm3 3h-2v-2h2v2zm0-3h-2V8h2v2z"),
    ("headphones", "Headphones", "M12 1c-4.97 0-9 4.03-9 9v7c0 1.66 1.34 3 3 3h3v-8H5v-2c0-3.87 3.13-7 7-7s7 3.13 7 7v2h-4v8h3c1.66 0 3-1.34 3-3v-7c0-4.97-4.03-9-9-9z"),
    ("mic", "Microphone", "M12 14c1.66 0 2.99-1.34 2.99-3L15 5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3zm5.3-3c0 3-2.54 5.1-5.3 5.1S6.7 14 6.7 11H5c0 3.41 2.72 6.23 6 6.72V21h2v-3.28c3.28-.48 6-3.3 6-6.72h-1.7z"),
    ("videocam", "Camera", "M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11l-4 4z"),
    ("photo", "Photo Camera", "M12 15.2a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4zM9 2L7.17 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2h-3.17L15 2H9zm3 15c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5z"),
    ("tv", "Screen", "M21 3H3c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h5v2h8v-2h5c1.1 0 1.99-.9 1.99-2L23 5c0-1.1-.9-2-2-2zm0 14H3V5h18v12z"),
    ("music", "Music", "M9 3v12.3A3.5 3.5 0 1 0 11 18V8h6V3z"),
    ("game", "Controller", "M7 6h10a5 5 0 0 1 0 10c-1.5 0-2.4-.7-3.2-1.6h-3.6C9.4 15.3 8.5 16 7 16A5 5 0 0 1 7 6zm-1 3v2H4v2h2v2h2v-2h2v-2H8V9z"),
    ("trophy", "Trophy", "M19 5h-2V3H7v2H5c-1.1 0-2 .9-2 2v1c0 2.55 1.92 4.63 4.39 4.94.63 1.5 1.98 2.63 3.61 2.96V19H7v2h10v-2h-4v-3.1c1.63-.33 2.98-1.46 3.61-2.96C19.08 12.63 21 10.55 21 8V7c0-1.1-.9-2-2-2zM5 8V7h2v3.82C5.84 10.4 5 9.3 5 8zm14 0c0 1.3-.84 2.4-2 2.82V7h2v1z"),
    ("crown", "Crown", "M3 18h18v2H3zM3 7l4.5 4L12 4l4.5 7L21 7l-2 10H5z"),
    ("star", "Star", "M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z"),
    ("bolt", "Lightning", "M7 2v11h3v9l7-12h-4l4-8z"),
    ("fire", "Fire", "M13.5.67s.74 2.65.74 4.8c0 2.06-1.35 3.73-3.41 3.73-2.07 0-3.63-1.67-3.63-3.73l.03-.36C5.21 7.51 4 10.62 4 14c0 4.42 3.58 8 8 8s8-3.58 8-8C20 8.61 17.41 3.8 13.5.67zM11.71 19c-1.78 0-3.22-1.4-3.22-3.14 0-1.62 1.05-2.76 2.81-3.12 1.77-.36 3.6-1.21 4.62-2.58.39 1.29.59 2.65.59 4.04 0 2.65-2.15 4.8-4.8 4.8z"),
    ("smile", "Smile", "M11.99 2C6.47 2 2 6.48 2 12s4.47 10 9.99 10C17.52 22 22 17.52 22 12S17.52 2 11.99 2zM12 20c-4.42 0-8-3.58-8-8s3.58-8 8-8 8 3.58 8 8-3.58 8-8 8zm3.5-9c.83 0 1.5-.67 1.5-1.5S16.33 8 15.5 8 14 8.67 14 9.5s.67 1.5 1.5 1.5zm-7 0c.83 0 1.5-.67 1.5-1.5S9.33 8 8.5 8 7 8.67 7 9.5 7.67 11 8.5 11zm3.5 6.5c2.33 0 4.31-1.46 5.11-3.5H6.89c.8 2.04 2.78 3.5 5.11 3.5z"),
    ("coffee", "Coffee", "M20 3H4v10c0 2.21 1.79 4 4 4h6c2.21 0 4-1.79 4-4v-3h2c1.11 0 2-.89 2-2V5c0-1.11-.89-2-2-2zm0 5h-2V5h2v3zM4 19h16v2H4z"),
    ("code", "Code", "M9.4 16.6L4.8 12l4.6-4.6L8 6l-6 6 6 6 1.4-1.4zm5.2 0l4.6-4.6-4.6-4.6L16 6l6 6-6 6-1.4-1.4z"),
    ("palette", "Art", "M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9c.83 0 1.5-.67 1.5-1.5 0-.39-.15-.74-.39-1.01-.23-.26-.38-.61-.38-.99 0-.83.67-1.5 1.5-1.5H16c2.76 0 5-2.24 5-5 0-4.42-4.03-8-9-8zm-5.5 9c-.83 0-1.5-.67-1.5-1.5S5.67 9 6.5 9 8 9.67 8 10.5 7.33 12 6.5 12zm3-4C8.67 8 8 7.33 8 6.5S8.67 5 9.5 5s1.5.67 1.5 1.5S10.33 8 9.5 8zm5 0c-.83 0-1.5-.67-1.5-1.5S13.67 5 14.5 5s1.5.67 1.5 1.5S15.33 8 14.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S16.67 9 17.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z"),
    ("book", "Book", "M18 2H6c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zM6 4h5v8l-2.5-1.5L6 12V4z"),
    ("globe", "Globe", "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 17.93c-3.95-.49-7-3.85-7-7.93 0-.62.08-1.21.21-1.79L9 15v1c0 1.1.9 2 2 2v1.93zm6.9-2.54c-.26-.81-1-1.39-1.9-1.39h-1v-3c0-.55-.45-1-1-1H8v-2h2c.55 0 1-.45 1-1V7h2c1.1 0 2-.9 2-2v-.41c2.93 1.19 5 4.06 5 7.41 0 2.08-.8 3.97-2.1 5.39z"),
    ("flag", "Flag", "M14.4 6L14 4H5v17h2v-7h5.6l.4 2h7V6z"),
    ("pulse", "Pulse", "M2 13h4l2.5-6 4 12 3-9 1.5 3H22v-2h-3.7L16 6.5l-3 9-4-12L5.3 11H2z"),
    ("omni", "OmniPulse", "M13.4 1.6c.7 0 1.2.5 1.3 1.1.4 2.3 1.9 3.8 3.4 5.2 1.9 1.7 2.6 4.3 1.5 6.6l-.9 1.6c-.2-.7 0-1.4.2-2 .6-1.5.1-3.1-1.1-4.1-1.1-.9-2.2-1.8-3.1-2.9v11.6c0 2.6-2.4 4.7-5.2 4.7S4.3 21.9 4.3 19.3s2.4-4.7 5.2-4.7c1 0 1.9.3 2.7.7V2.9c0-.7.5-1.3 1.2-1.3z"),
];

/// The icon a panel's title picks (its words: About a person, Rules a shield, and so on; else a star)
pub fn for_title(title: &str) -> &'static str {
    let t = title.to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|w| t.contains(w));
    if has(&["omnipulse"]) { return "omni"; }
    if has(&["about", "me", "bio", "who"]) { return "user"; }
    if has(&["schedule", "times", "when", "days"]) { return "calendar"; }
    if has(&["rule"]) { return "shield"; }
    if has(&["social", "follow", "twitter", "insta", "tiktok", "link"]) { return "link"; }
    if has(&["donat", "tip", "support", "sub"]) { return "heart"; }
    if has(&["discord", "chat", "command"]) { return "chat"; }
    if has(&["merch", "shop", "store"]) { return "bag"; }
    if has(&["setup", "pc", "spec", "gear"]) { return "pc"; }
    if has(&["music", "song", "request"]) { return "music"; }
    if has(&["faq", "question", "help"]) { return "question"; }
    if has(&["game", "play"]) { return "game"; }
    if has(&["powered"]) { return "pulse"; }
    "star"
}

/// An icon's shape, by its key (a star if there's no such icon)
pub fn shape(key: &str) -> &'static str {
    ICONS.iter().find(|i| i.0 == key).map_or(ICONS.iter().find(|i| i.0 == "star").map_or("", |i| i.2), |i| i.2)
}

/// A panel's icon: the one chosen, else the one its title picks
pub fn of(key: &str, title: &str) -> &'static str {
    if ICONS.iter().any(|i| i.0 == key) { ICONS.iter().find(|i| i.0 == key).map_or("star", |i| i.0) } else { for_title(title) }
}

/// A picture made into an icon: its background cut out (where it's see-through already, that; else whatever's
/// like its edges' colour), trimmed to what's left and centred in a square, all of it white, kept as a PNG
pub fn mask(img: &image::DynamicImage) -> image::RgbaImage {
    let src = img.thumbnail(512, 512).to_rgba8();
    let (w, h) = src.dimensions();
    let n = (w * h).max(1) as usize;
    // (see-through in places already: its own see-through used)
    let clear = src.pixels().filter(|p| p[3] < 128).count();
    let alpha: Vec<u8> = if clear * 100 / n >= 3 {
        src.pixels().map(|p| p[3]).collect()
    } else {
        // (its background: the average of its edges' colour; each pixel as solid as it's unlike that)
        let mut edge = Vec::new();
        for x in 0..w { edge.push(*src.get_pixel(x, 0)); edge.push(*src.get_pixel(x, h - 1)); }
        for y in 0..h { edge.push(*src.get_pixel(0, y)); edge.push(*src.get_pixel(w - 1, y)); }
        let avg = |k: usize| edge.iter().map(|p| p[k] as f32).sum::<f32>() / edge.len().max(1) as f32;
        let bg = [avg(0), avg(1), avg(2)];
        src.pixels().map(|p| {
            let d = ((p[0] as f32 - bg[0]).powi(2) + (p[1] as f32 - bg[1]).powi(2) + (p[2] as f32 - bg[2]).powi(2)).sqrt();
            // (a soft edge between nearly the background (40) and plainly not (90))
            let t = ((d - 40.0) / 50.0).clamp(0.0, 1.0);
            (t * t * (3.0 - 2.0 * t) * p[3] as f32) as u8
        }).collect()
    };
    // (trimmed to what's left, with a little room round it)
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h { for x in 0..w { if alpha[(y * w + x) as usize] > 24 { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y); } } }
    if x1 < x0 { x0 = 0; y0 = 0; x1 = w - 1; y1 = h - 1; }
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    let side = bw.max(bh) + bw.max(bh) / 12 * 2;
    let (ox, oy) = ((side - bw) / 2, (side - bh) / 2);
    let mut out = image::RgbaImage::from_pixel(side, side, image::Rgba([255, 255, 255, 0]));
    for y in 0..bh { for x in 0..bw { out.put_pixel(ox + x, oy + y, image::Rgba([255, 255, 255, alpha[((y0 + y) * w + x0 + x) as usize]])); } }
    image::imageops::resize(&out, 256, 256, image::imageops::FilterType::Lanczos3)
}

/// A picture of yours (its address on the widgets' server, /overlay/...) made into an icon: kept beside it as
/// another picture there (named after it, made once), and that one's address
pub fn mask_of(url: &str) -> Result<String, String> {
    let name = url.strip_prefix("/overlay/").filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') && !n.starts_with('.')).ok_or("only a picture you've added can be made into an icon")?;
    let dir = omnipulse_core::data_dir()?.join("overlay-media");
    let stem = name.split('.').next().unwrap_or(name);
    let out = format!("{stem}icon.png");
    let to = dir.join(&out);
    if !to.is_file() {
        let img = image::open(dir.join(name)).map_err(|e| format!("couldn't read that picture: {e}"))?;
        mask(&img).save(&to).map_err(|e| format!("couldn't make the icon: {e}"))?;
    }
    Ok(format!("/overlay/{out}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logo_on_white_becomes_an_icon() {
        // (a dark square on white: the square kept, solid; the white gone; trimmed and centred)
        let mut img = image::RgbaImage::from_pixel(200, 100, image::Rgba([255, 255, 255, 255]));
        for y in 30..70 { for x in 120..160 { img.put_pixel(x, y, image::Rgba([20, 30, 200, 255])); } }
        let m = mask(&image::DynamicImage::ImageRgba8(img));
        assert_eq!(m.dimensions(), (256, 256));
        assert!(m.get_pixel(128, 128)[3] > 240);
        assert!(m.get_pixel(5, 5)[3] < 10);
        assert_eq!(m.get_pixel(128, 128)[0], 255);
    }

    /// (by hand: MASK_IN a picture, MASK_OUT where its icon goes)
    #[test]
    #[ignore]
    fn mask_a_picture() {
        let (Some(i), Some(o)) = (std::env::var_os("MASK_IN"), std::env::var_os("MASK_OUT")) else { return };
        mask(&image::open(i).unwrap()).save(o).unwrap();
    }

    #[test]
    fn titles_pick_icons_and_keys_are_known() {
        assert_eq!(for_title("About Me"), "user");
        assert_eq!(for_title("PC Specs"), "pc");
        assert_eq!(of("trophy", "About Me"), "trophy");
        assert_eq!(of("", "Rules"), "shield");
        assert_eq!(of("nope", "Rules"), "shield");
        assert!(shape("headphones").starts_with('M'));
        let mut keys: Vec<_> = ICONS.iter().map(|i| i.0).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ICONS.len());
    }
}
