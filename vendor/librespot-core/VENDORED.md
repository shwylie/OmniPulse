# librespot-core 0.8.0, vendored

Copied from crates.io (librespot-core 0.8.0) with one change, in `src/session.rs`
(`Session::check_catalogue`): an account that isn't Premium is logged, not `exit(1)`. Upstream
exits the whole process there ("TODO: logout instead of exiting"), which closed OmniPulse when
someone with a free Spotify account used the playback sign-in, with nothing to catch it. OmniPulse
reads the account type right after connecting and signs a free account out of playback itself.

Used through `[patch.crates-io]` in the workspace's Cargo.toml. To update librespot, copy the new
version's crate here and make the same change (or drop this if upstream logs out instead).
