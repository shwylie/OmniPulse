// Your own keys for Spotify, Last.fm and a YouTube account (as the web app's js/apisetup.js): each
// platform gives them out free, from its developer pages, and none lets an app make them for you. So
// this says what to do on each page (opened in your browser, where you're signed in already), with
// the values to copy, and takes the keys you paste back. They're kept in config.json on this PC
// (core::config_set), never shown again after.

use regex::Regex;

pub struct Step {
    pub text: &'static str,
    pub url: Option<&'static str>,
    /// (what it's for, the value to copy)
    pub copies: Vec<(&'static str, String)>,
}

pub struct Setup {
    pub name: &'static str,
    /// (config key, label, what a right one looks like)
    pub fields: Vec<(&'static str, &'static str, &'static str)>,
    pub steps: Vec<Step>,
    pub note: &'static str,
}

/// The guide for a platform ("spotify", "lastfm", "youtube")
pub fn setup(plat: &str) -> Option<Setup> {
    let st = omnipulse_core::config_status();
    Some(match plat {
        "spotify" => Setup {
            name: "Spotify",
            fields: vec![("spotify_client_id", "Client ID", r"^[0-9a-fA-F]{32}$")],
            steps: vec![
                Step { text: "Open the Spotify Developer Dashboard and sign in. If it asks, accept the developer terms.", url: Some("https://developer.spotify.com/dashboard"), copies: vec![] },
                Step { text: "Create an app. Name and description: anything (say, OmniPulse). Redirect URI: the address below, then Add. Tick Web API and Web Playback SDK, agree to the terms, and Save.", url: Some("https://developer.spotify.com/dashboard/create"),
                    copies: vec![("App name", "OmniPulse".into()), ("Redirect URI", st["spotify_redirect"].as_str().filter(|s| !s.is_empty()).unwrap_or("http://127.0.0.1:8081/callback").to_string())] },
                Step { text: "On your app's page, open Settings and copy its Client ID. There's no secret to copy: OmniPulse signs in without one.", url: None, copies: vec![] },
            ],
            note: "With Premium, this app can be shared: friends with a free account can sign in with it too. On its page in the Developer Dashboard, open User Management and add each one (their name, and their Spotify account's email; up to five), then send them its Client ID.",
        },
        // a free account: Spotify only lets an app work for its owner with Premium (since February 2026),
        // so one of its own can't be; it signs in with the app of someone with Premium who's added it
        "spotify-free" => Setup {
            name: "Spotify",
            fields: vec![("spotify_client_id", "Client ID", r"^[0-9a-fA-F]{32}$")],
            steps: vec![
                Step { text: "Ask someone with Spotify Premium (a friend who uses OmniPulse, say) to add you to their Spotify app. On its page in the Spotify Developer Dashboard they open User Management and add your name and the email of your Spotify account. (No app yet? With Premium, they make one in OmniPulse: Settings > Accounts > Spotify's keys.)", url: Some("https://developer.spotify.com/dashboard"), copies: vec![] },
                Step { text: "They send you its Client ID. It isn't a secret: OmniPulse signs in without one.", url: None, copies: vec![] },
                Step { text: "Paste it here and save: OmniPulse signs you in with their app.", url: None, copies: vec![] },
            ],
            note: "Spotify only lets an app work when its owner has Premium (since February 2026), so a free account can't use keys of its own. An app can have up to five people added to it. Spotify songs play from YouTube Music.",
        },
        "lastfm" => Setup {
            name: "Last.fm",
            fields: vec![("lastfm_api_key", "API Key", r"^[0-9a-fA-F]{32}$"), ("lastfm_secret", "Shared Secret", r"^[0-9a-fA-F]{32}$")],
            steps: vec![
                Step { text: "Open Last.fm's API account page and sign in.", url: Some("https://www.last.fm/api/account/create"), copies: vec![] },
                Step { text: "Fill in an application name and description (anything: say, OmniPulse). The callback URL can stay empty. Submit.", url: None, copies: vec![("Application name", "OmniPulse".into())] },
                Step { text: "Copy the API key and the shared secret it shows you.", url: None, copies: vec![] },
            ],
            note: "Last.fm shows them once on that page; it also lists them under your API accounts later.",
        },
        "youtube" => Setup {
            name: "YouTube",
            fields: vec![("youtube_client_id", "Client ID", r"^\d+-[0-9a-z]+\.apps\.googleusercontent\.com$"), ("youtube_client_secret", "Client Secret", r"^GOCSPX-[\w-]{10,}$")],
            steps: vec![
                Step { text: "Open Google Cloud and create a project (any name: say, OmniPulse).", url: Some("https://console.cloud.google.com/projectcreate"), copies: vec![] },
                Step { text: "Switch on the YouTube Data API v3 for it.", url: Some("https://console.cloud.google.com/apis/library/youtube.googleapis.com"), copies: vec![] },
                Step { text: "Set up its sign-in screen: app name OmniPulse, your email, audience External. Under Audience, add your own Google account as a test user.", url: Some("https://console.cloud.google.com/auth/overview"), copies: vec![] },
                Step { text: "Create an OAuth client: application type Desktop app, any name. Copy its Client ID and Client secret.", url: Some("https://console.cloud.google.com/auth/clients/create"), copies: vec![] },
            ],
            note: "It's free. Google may say the app isn't verified when you sign in: it's yours, so that's expected.",
        },
        _ => return None,
    })
}

/// Whether a typed key looks right
pub fn looks_right(pattern: &str, value: &str) -> bool {
    Regex::new(pattern).is_ok_and(|r| r.is_match(value.trim()))
}

/// Whether a platform's keys are set already
pub fn has_keys(plat: &str) -> bool {
    omnipulse_core::config_status()[plat].as_bool() == Some(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys_are_checked() {
        let sp = super::setup("spotify").unwrap();
        assert!(super::looks_right(sp.fields[0].2, "0123456789abcdef0123456789ABCDEF"));
        assert!(!super::looks_right(sp.fields[0].2, "nope"));
        let yt = super::setup("youtube").unwrap();
        assert!(super::looks_right(yt.fields[0].2, "1234567890-abcdefghijklmnop.apps.googleusercontent.com"));
        assert!(super::looks_right(yt.fields[1].2, "GOCSPX-abcdefghijklmno"));
        assert!(!super::looks_right(yt.fields[1].2, "secret"));
    }
}
