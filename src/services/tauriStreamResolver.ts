/**
 * Stream resolution for the UI: yt-dlp through the app's Rust side first (resolve_yt_stream in
 * src-tauri/src/yt_stream.rs), and the Piped instances (./streamResolver) if that fails, for
 * example when yt-dlp isn't installed.
 *
 *   import { resolveStream } from "./services/tauriStreamResolver";
 *   const url = await resolveStream("Daft Punk", "Get Lucky");
 *
 * Note: the app's current pages (dist/) are plain scripts that use window.__TAURI__ and don't load
 * this module; it's for a bundled (Vite etc.) frontend that imports @tauri-apps/api.
 */

import { invoke } from "@tauri-apps/api/core";
import { createStreamResolver, StreamResolveError } from "./streamResolver";

const piped = createStreamResolver();

/** "Artist - Title", as both yt-dlp and Piped search it. */
const searchFor = (artist: string, title: string) => [artist.trim(), title.trim()].filter(Boolean).join(" - ");

/**
 * The direct URL of the best audio stream for a song. Throws if neither yt-dlp nor any Piped
 * instance can find one; the error says what each of them reported.
 */
export async function resolveStream(artist: string, title: string): Promise<string> {
  const query = searchFor(artist, title);
  if (!query) throw new Error("resolveStream needs an artist or a title.");

  let ytdlpError: string;
  try {
    const url = await invoke<string>("resolve_yt_stream", { query });
    if (url) return url;
    ytdlpError = "yt-dlp returned no URL";
  } catch (e) {
    // invoke rejects with the Rust command's error string (or an Error outside Tauri)
    ytdlpError = e instanceof Error ? e.message : String(e);
  }
  console.warn(`[tauriStreamResolver] yt-dlp didn't resolve "${query}" (${ytdlpError}). Trying Piped.`);

  try {
    return await piped.resolveStream(artist, title);
  } catch (e) {
    const pipedError = e instanceof Error ? e.message : String(e);
    throw new StreamResolveError(
      `Couldn't find a stream for "${query}". yt-dlp: ${ytdlpError.replace(/\.$/, "")}. Piped: ${pipedError}`,
      e instanceof StreamResolveError ? e.attempts : [],
      e instanceof StreamResolveError ? e.notFound : false,
    );
  }
}
