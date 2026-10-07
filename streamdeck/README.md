# SC Client for Stream Deck

Keys for SC Client: Play, Pause, Play / Pause, Next, Previous, Like, Favourite, Shuffle, Repeat,
Clear requests, Clear queue, and Sets & podcasts (lets them in or keeps them out of the queue and autoplay).
Play / Pause, Like, Favourite, Shuffle, Repeat and Sets & podcasts light up to show what the app is doing.

## Install

1. In SC Client, open Settings > App > Stream Deck and switch it on.
2. Install Node.js 20 or newer, then in this folder:

   ```
   cd com.shwylie.scclient.sdPlugin
   npm install
   ```

3. Quit the Stream Deck app, copy (or link) `com.shwylie.scclient.sdPlugin` into
   `%APPDATA%\Elgato\StreamDeck\Plugins\`, and start Stream Deck again. A link keeps it up to date:

   ```
   mklink /J "%APPDATA%\Elgato\StreamDeck\Plugins\com.shwylie.scclient.sdPlugin" "C:\dev\sc-client\streamdeck\com.shwylie.scclient.sdPlugin"
   ```

   (Or, with the Elgato CLI: `npm i -g @elgato/cli`, then `streamdeck link com.shwylie.scclient.sdPlugin`.)

4. Drag keys from the "SC Client" category.

There is nothing to set up in the plugin: SC Client writes its port and secret key to
`%APPDATA%\sc-client\remote.json` while Stream Deck control is on, and the plugin reads it on every press.
A key shows a warning triangle when the app isn't running or control is switched off.

## Without the plugin

Anything that can open a URL works too (Stream Deck's Website action in the background, Touch Portal,
AutoHotkey): copy the URL from Settings and change `action=` to any of the actions above
(`play`, `pause`, `toggle`, `next`, `previous`, `like`, `favourite`, `shuffle`, `repeat`,
`clear-requests`, `clear-queue`, `sets`). `/state?key=...` answers with what's playing. It only listens on
this PC (127.0.0.1).
