// Keys that belong to the app, not to you: built in, so nothing needs setting up for them.
//   - SoundCloud's client id and secret: scrambled into the build from keys.local.json (see build.rs).
//     Harder to find, not impossible: the sign-in sends the secret to SoundCloud, so it can be seen
//     by someone watching their own traffic. A server doing that exchange is the lasting fix.
//   - Discord's application id: public by design (it's in every Rich Presence connection), so it's
//     simply written here.
// Keys you add to config.json win over these.

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

pub const DISCORD_CLIENT_ID: &str = "1556765709671862284";

fn unscramble(enc: &[u8], key: &[u8]) -> String {
    let n = enc.len();
    let bytes: Vec<u8> = (0..n)
        .map(|i| std::hint::black_box(enc[n - 1 - i]) ^ key[i] ^ ((i as u8).wrapping_mul(0x5d).wrapping_add(0xa7)))
        .collect();
    String::from_utf8(bytes).unwrap_or_default()
}

/// SoundCloud's client id, secret and sign-in address, if this build has them
pub fn soundcloud() -> Option<(String, String, String)> {
    let id = unscramble(&SC_ID_E, &SC_ID_K);
    let secret = unscramble(&SC_SECRET_E, &SC_SECRET_K);
    if id.is_empty() || secret.is_empty() {
        return None;
    }
    let redirect = unscramble(&SC_REDIRECT_E, &SC_REDIRECT_K);
    Some((id, secret, if redirect.is_empty() { crate::default_redirect() } else { redirect }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn scrambled_keys_come_back() {
        // whatever this build has, it reads back as it went in: whole, or nothing at all
        if let Some((id, secret, redirect)) = super::soundcloud() {
            assert!(id.is_ascii() && secret.is_ascii() && !id.is_empty() && !secret.is_empty());
            assert!(redirect.starts_with("http"));
        }
    }
}
