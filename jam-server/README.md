# OmniPulse relay: Listen Along and Jams

A small Cloudflare Worker that lets friends listen with you in their own OmniPulse. Your app sends
what it's playing (the track, where it's at, the queue coming up, your start/end times). Their apps
play the same songs through their own accounts, with their own EQ and loudness. No audio goes
through it, only those small updates, and Cloudflare's free plan covers it easily.

It also serves the page the Discord **Listen along** button and Jam invites open: it opens OmniPulse
if it's installed, and offers to download it if it isn't.

## Set it up (once, about 10 minutes)

1. Make a free Cloudflare account at https://dash.cloudflare.com/sign-up.
2. In this folder, run these. The first command opens your browser to sign in to Cloudflare:
   ```
   npm install
   npx wrangler login
   npx wrangler deploy
   ```
   It prints your relay's address, like `https://omnipulse-jam.<your-name>.workers.dev`.
3. In OmniPulse: **Settings > Streaming > Listen Along & Jams > Relay Address**, paste that address.

It also keeps **shared playlists**: a playlist page's **Share** button puts the playlist here, as a page listing each song with its links on SoundCloud, Spotify and YouTube Music, and **Open in OmniPulse** to add it. Already set up the relay before this? Run `npx wrangler deploy` in this folder again to add it.

That's all. With **Discord Status** and its **Listen Along Button** on, your status gets a "Listen
along" button while you play. The queue panel has **Start a Jam** for invite-only sessions.

## The download link

The join page's **Download OmniPulse** button points at `DOWNLOAD_URL`. Set it to wherever you put
the installer, for example a GitHub release's `OmniPulse_x64-setup.exe`. Change it any time:

- in the Cloudflare dashboard: Workers & Pages > omnipulse-jam > Settings > Variables and Secrets, or
- in `wrangler.toml` under `[vars]`, then `npx wrangler deploy` again.

If it's empty, the page only shows "Open in OmniPulse".

## Discord

Nothing to change in the Discord Developer Portal. The status buttons are plain links to your
relay's page, which Discord shows to everyone who can see your status. Discord can't host a download
page or open apps by itself, so the relay's page does that.

## How it's kept to the people you mean

- **Listen Along** (from your Discord status): anyone who can see your status can listen. They can't
  change or add anything, and they see only what your status already shows.
- **Jam**: only people with the invite link can join. Members can add songs, which show as theirs in
  your queue, and you can remove them. Ending the Jam sends everyone back to their own music.
- A room nobody has used for 12 hours goes away by itself. Rooms hold only the latest "now playing"
  update, never account details.

## Trying it on this PC

`npx wrangler dev --port 8787` runs it locally. Set the Relay Address to `http://127.0.0.1:8787`.

## Sign-ins that need a secret (SoundCloud, Kick)

OmniPulse's own SoundCloud and Kick apps need their secret to finish a sign-in or renew one. The secrets
live here, not in the app: the app sends the sign-in's code (or its refresh token) to `/api/soundcloud/*`
or `/api/kick/*`, and the relay adds the app's keys and asks SoundCloud or Kick.

The Client IDs are in `wrangler.toml` (`SC_CLIENT_ID`, `KICK_CLIENT_ID`). Set the secrets once, yourself:

    npx wrangler secret put SC_CLIENT_SECRET
    npx wrangler secret put KICK_CLIENT_SECRET

Until a secret is set, its sign-ins answer 503 ("isn't set up on this relay").
