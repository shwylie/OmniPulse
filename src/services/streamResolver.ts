/**
 * Finds a direct audio stream for a song through Piped (an open-source YouTube front end),
 * failing over between Piped instances.
 *
 *   import { resolveStream } from "./services/streamResolver";
 *   const url = await resolveStream("Daft Punk", "Get Lucky");
 *
 * For each instance in turn: search `${artist} - ${title}` with filter=music_songs, take the top
 * song's video id, read /streams/:id, and pick the highest-bitrate M4A or WebM audio stream. If
 * an instance fails, times out or answers with a non-200 status, a warning is logged and the
 * whole sequence is tried on the next instance. If every instance fails, a StreamResolveError
 * lists what went wrong on each.
 *
 * Note: stream URLs are signed by YouTube and expire after a few hours, so resolve them close to
 * playback rather than storing them.
 */

export const DEFAULT_INSTANCES = [
  "https://pipedapi.kavin.rocks",
  "https://api.piped.video",
  "https://pipedapi.adminforge.de",
];

export interface StreamResolverOptions {
  /** Piped API instances, tried in this order. */
  instances?: string[];
  fetch?: typeof fetch;
  /** How long each request may take before the instance counts as failed (default 8 s). */
  timeoutMs?: number;
  /** Where failover warnings go (default console.warn). */
  logger?: (message: string) => void;
}

export interface ResolvedStream {
  url: string;
  /** e.g. "audio/mp4; codecs=\"mp4a.40.2\"" or "audio/webm; codecs=\"opus\"" */
  mimeType: string;
  /** Bits per second, as reported by Piped. */
  bitrate: number;
  videoId: string;
  title: string;
  /** The instance that answered. */
  instance: string;
}

export interface InstanceAttempt {
  instance: string;
  reason: string;
}

export class StreamResolveError extends Error {
  constructor(
    message: string,
    /** What went wrong on each instance, in the order they were tried. */
    readonly attempts: InstanceAttempt[],
    /** True when every instance answered but none found the song. */
    readonly notFound: boolean,
  ) {
    super(message);
    this.name = "StreamResolveError";
  }
}

// ---------- Piped's response shapes (only what's used) ----------
interface PipedSearchItem {
  url?: string;   // "/watch?v=VIDEO_ID"
  type?: string;  // "stream" for videos and songs
  title?: string;
}
interface PipedAudioStream {
  url?: string;
  mimeType?: string;
  format?: string;     // "M4A", "WEBMA_OPUS", ...
  bitrate?: number;
  videoOnly?: boolean;
  audioTrackType?: string | null;  // "ORIGINAL", "DUBBED", "DESCRIPTIVE", or none
}
interface PipedStreams {
  title?: string;
  audioStreams?: PipedAudioStream[];
}

/** A failure that moves on to the next instance; notFound marks "worked, but no such song". */
class InstanceFailure extends Error {
  constructor(message: string, readonly notFound = false) {
    super(message);
  }
}

const trimSlash = (u: string) => u.replace(/\/+$/, "");

function videoIdOf(item: PipedSearchItem): string | null {
  if (!item.url || (item.type && item.type !== "stream")) return null;
  const m = item.url.match(/[?&]v=([\w-]{11})/) ?? item.url.match(/^\/?([\w-]{11})$/);
  return m ? m[1] : null;
}

// An audio-only M4A or WebM stream (going by its MIME type, or its format name if it has none)
function isWanted(s: PipedAudioStream): boolean {
  if (!s.url || s.videoOnly) return false;
  if (s.mimeType) return /^audio\/(mp4|webm)\b/i.test(s.mimeType);
  return /^(M4A|WEBMA)/i.test(s.format ?? "");
}

/** The best M4A or WebM audio stream: original-language tracks first, then highest bitrate. */
export function pickAudioStream(streams: PipedAudioStream[]): PipedAudioStream | null {
  const usable = streams.filter(isWanted);
  if (!usable.length) return null;
  const original = usable.filter((s) => !s.audioTrackType || /^original$/i.test(s.audioTrackType));
  const pool = original.length ? original : usable;
  return pool.reduce((best, s) => ((s.bitrate ?? 0) > (best.bitrate ?? 0) ? s : best));
}

export function createStreamResolver(options: StreamResolverOptions = {}) {
  const instances = (options.instances ?? DEFAULT_INSTANCES).map(trimSlash).filter(Boolean);
  if (!instances.length) throw new Error("createStreamResolver needs at least one Piped instance.");
  // looked up on each call, so a fetch replaced after import (a polyfill, a test) is used
  const doFetch: typeof fetch = (input, init) => (options.fetch ?? globalThis.fetch)(input, init);
  const timeoutMs = options.timeoutMs ?? 8000;
  const warn = options.logger ?? ((m: string) => console.warn(m));

  /** GET JSON from an instance; anything but a 200 with JSON counts as that instance failing. */
  async function getJson<T>(url: string): Promise<T> {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), timeoutMs);
    try {
      const res = await doFetch(url, { headers: { Accept: "application/json" }, signal: ctrl.signal });
      if (res.status !== 200) {
        // Piped often explains itself in the body ({"error": "..."}); include the first line of it
        const why = await res.text().then((t) => { try { return String(JSON.parse(t).error ?? "").split("\n")[0]; } catch { return ""; } }).catch(() => "");
        throw new InstanceFailure(`HTTP ${res.status} from ${new URL(url).pathname}` + (why ? `: ${why.slice(0, 200)}` : ""));
      }
      try {
        return (await res.json()) as T;
      } catch {
        throw new InstanceFailure(`a response from ${new URL(url).pathname} that wasn't JSON`);
      }
    } catch (e) {
      if (e instanceof InstanceFailure) throw e;
      if (ctrl.signal.aborted) throw new InstanceFailure(`timed out after ${timeoutMs} ms`);
      throw new InstanceFailure(`couldn't connect (${(e as Error)?.message ?? e})`);
    } finally {
      clearTimeout(timer);
    }
  }

  async function resolveOn(instance: string, query: string): Promise<ResolvedStream> {
    const search = await getJson<{ items?: PipedSearchItem[] }>(
      `${instance}/search?q=${encodeURIComponent(query)}&filter=music_songs`);
    const top = (search.items ?? []).map((item) => ({ item, id: videoIdOf(item) })).find((x) => x.id);
    if (!top) throw new InstanceFailure(`no songs found for "${query}"`, true);
    const streams = await getJson<PipedStreams & { error?: string }>(`${instance}/streams/${top.id}`);
    if (streams.error && !streams.audioStreams?.length) {
      // e.g. YouTube asking the instance to "sign in to confirm you're not a bot"
      throw new InstanceFailure(`Piped couldn't read video ${top.id}: ${streams.error.split("\n")[0].slice(0, 200)}`);
    }
    const best = pickAudioStream(streams.audioStreams ?? []);
    if (!best) throw new InstanceFailure(`no M4A or WebM audio stream for video ${top.id}`);
    return {
      url: best.url!,
      mimeType: best.mimeType ?? (/^M4A/i.test(best.format ?? "") ? "audio/mp4" : "audio/webm"),
      bitrate: best.bitrate ?? 0,
      videoId: top.id!,
      title: streams.title ?? top.item.title ?? "",
      instance,
    };
  }

  /** The best audio stream for a song, with details about where it came from. */
  async function resolveStreamInfo(artist: string, title: string): Promise<ResolvedStream> {
    const query = `${artist.trim()} - ${title.trim()}`;
    if (!artist.trim() && !title.trim()) throw new StreamResolveError("resolveStream needs an artist or a title.", [], true);
    const attempts: InstanceAttempt[] = [];
    let allNotFound = true;
    for (const [i, instance] of instances.entries()) {
      try {
        return await resolveOn(instance, query);
      } catch (e) {
        const failure = e instanceof InstanceFailure ? e : new InstanceFailure(String((e as Error)?.message ?? e));
        allNotFound &&= failure.notFound;
        attempts.push({ instance, reason: failure.message });
        const next = instances[i + 1];
        warn(`[streamResolver] ${instance} failed: ${failure.message}.` + (next ? ` Trying ${next}.` : ""));
      }
    }
    const detail = attempts.map((a) => `${a.instance}: ${a.reason}`).join("; ");
    throw new StreamResolveError(
      allNotFound
        ? `No song found for "${query}" on any Piped instance (${detail}).`
        : `Couldn't resolve a stream for "${query}": all ${instances.length} Piped instances failed (${detail}).`,
      attempts,
      allNotFound,
    );
  }

  /** The direct URL of the best M4A or WebM audio stream for a song. */
  async function resolveStream(artist: string, title: string): Promise<string> {
    return (await resolveStreamInfo(artist, title)).url;
  }

  return { resolveStream, resolveStreamInfo, instances: [...instances] };
}

// The default resolver, using DEFAULT_INSTANCES
const defaultResolver = createStreamResolver();

export const resolveStream = defaultResolver.resolveStream;
export const resolveStreamInfo = defaultResolver.resolveStreamInfo;
