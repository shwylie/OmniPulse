# OmniPulse's display for Cast screens

Casting works without this. Screens then show Google's own player, with the title, artist and cover.

This page is OmniPulse's own display: what's playing in the style of the OBS widget, with the colours
from the cover, a progress bar, and play/pause and skip buttons on touch screens (a Nest Hub). It has
no browsing; the app on the PC stays the player.

Google only opens custom displays that are registered to someone and served over https. To set it up:

1. Put `index.html` somewhere served over https. GitHub Pages is free: a repository with this file,
   then Settings > Pages. Note its address, for example `https://you.github.io/omnipulse-cast/`.
2. Register with the Google Cast SDK Developer Console (https://cast.google.com/publish). It's a
   one-time US$5 fee.
3. Add a new application of type **Custom Receiver**, with the address from step 1. You'll get an
   **application ID** (8 letters and digits).
4. Until the app is published there, it only opens on devices you add under **Devices** with their
   serial numbers. Restart the device after adding it. You can also publish it, which makes it work
   on any device.
5. In OmniPulse, enter the ID in **Settings > Sound > Casting > Display on screens**.

Speakers without a screen always use Google's player, which only plays the sound.
