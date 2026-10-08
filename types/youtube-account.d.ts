// TypeScript definitions for the optional YouTube account's Tauri commands (src-tauri/src/main.rs,
// crates/core/src/yt_account.rs). The app's own pages are plain JavaScript (dist/js/youtube.js uses
// these); this is for anything written in TypeScript against them, like a plugin.
//
//   import { invoke } from "@tauri-apps/api/core";
//   const s = await invoke<YoutubeStatus>("youtube_status");

/** Whether an account is connected, and its channel's name */
export type YoutubeStatus =
  | { connected: true; name: string }
  /** configured: false when config.json has no youtube_client_id / youtube_client_secret */
  | { connected: false; configured: boolean };

/** What liking and unliking take: the video, if known; otherwise it's found by the song's name */
export interface YoutubeRateArgs {
  /** a YouTube (or YouTube Music) video id; null to look the song up by title and artist */
  videoId: string | null;
  title: string;
  artist: string;
}

export interface YoutubeCommands {
  youtube_status(): Promise<YoutubeStatus>;
  /** opens Google's sign-in in the browser and resolves once you've come back (up to 3 minutes) */
  youtube_connect(): Promise<YoutubeStatus>;
  /** signs out here and tells Google to forget the sign-in */
  youtube_disconnect(): Promise<void>;
  /** likes it on YouTube (and so YouTube Music); resolves with the video id used */
  like_youtube_track(args: YoutubeRateArgs): Promise<string>;
  /** takes the like back (rating: none); resolves with the video id used */
  unlike_youtube_track(args: YoutubeRateArgs): Promise<string>;
  /** each video's licence, for Stream-Safe Music (videos YouTube doesn't know are left out) */
  youtube_licences(args: { ids: string[] }): Promise<Record<string, "creativeCommon" | "youtube">>;
}

/** A typed invoke for these commands */
export type InvokeYoutube = <K extends keyof YoutubeCommands>(
  cmd: K,
  ...args: Parameters<YoutubeCommands[K]> extends [] ? [] : [Parameters<YoutubeCommands[K]>[0]]
) => ReturnType<YoutubeCommands[K]>;

/*
  The pattern the app uses (dist/js/youtube.js), for a heart button of your own:

  const status = await invoke<YoutubeStatus>("youtube_status");
  async function onHeart(track: { ytId?: string; title: string; artist: string }, like: boolean) {
    // the like on this PC happens either way; YouTube only when connected
    if (!status.connected) return;
    try {
      await invoke(like ? "like_youtube_track" : "unlike_youtube_track",
        { videoId: track.ytId ?? null, title: track.title, artist: track.artist });
    } catch (e) {
      showStatus("Liked here, but not on YouTube: " + e);
    }
  }
*/
