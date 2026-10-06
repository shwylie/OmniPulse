import { useEffect, useMemo, useRef, useState } from "react";

// Streams a { title, streamUrl } payload through an HTML5 <audio> element.
//
//   <StreamPlayer payload={{ title: "Live set", streamUrl: "https://example.com/stream.mp3" }} />
//   <StreamPlayer payload='{"title":"Radio","streamUrl":"https://example.com/live.m3u8"}' autoPlay />
//
// `payload` may be an object or a JSON string. Plain streams (MP3, AAC, Ogg, WAV, Icecast...)
// play natively. HLS (.m3u8) plays natively where the browser supports it (Safari) and
// otherwise through hls.js, loaded on demand only if installed (`npm install hls.js`).

const ERROR_TEXT = {
  1: "Playback was stopped.",
  2: "A network error interrupted the stream.",
  3: "The stream could not be decoded.",
  4: "This stream's format isn't supported, or the address couldn't be reached.",
};

// Returns { title, streamUrl } or { error }
export function parsePayload(payload) {
  let data = payload;
  if (typeof data === "string") {
    try {
      data = JSON.parse(data);
    } catch {
      return { error: "The payload isn't valid JSON." };
    }
  }
  if (!data || typeof data !== "object") return { error: "The payload must be an object with a streamUrl." };
  const { title, streamUrl } = data;
  if (typeof streamUrl !== "string" || !streamUrl.trim()) return { error: "The payload has no streamUrl." };
  let url;
  try {
    url = new URL(streamUrl.trim(), window.location.href);
  } catch {
    return { error: "The streamUrl isn't a valid address." };
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return { error: "Only http and https streams are allowed." };
  }
  return {
    title: typeof title === "string" && title.trim() ? title.trim() : "Untitled stream",
    streamUrl: url.href,
  };
}

const isHls = (url) => /\.m3u8(\?|#|$)/i.test(url);

export default function StreamPlayer({ payload, autoPlay = false, onError, onEnded, className }) {
  const audioRef = useRef(null);
  const [status, setStatus] = useState("idle"); // idle | loading | playing | paused | ended | error
  const [message, setMessage] = useState("");

  const parsed = useMemo(() => parsePayload(payload), [payload]);
  const { title, streamUrl, error: payloadError } = parsed;

  // Keep the latest callbacks without restarting the stream when a parent re-renders
  const onErrorRef = useRef(onError);
  const onEndedRef = useRef(onEnded);
  useEffect(() => { onErrorRef.current = onError; onEndedRef.current = onEnded; });

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return undefined;

    let failed = false;
    const fail = (text) => {
      failed = true;
      setStatus("error");
      setMessage(text);
      if (onErrorRef.current) onErrorRef.current(new Error(text));
    };

    if (payloadError) {
      fail(payloadError);
      return undefined;
    }

    let cancelled = false;
    let hls = null;
    setStatus(autoPlay ? "loading" : "idle");   // preload="none": nothing loads until play
    setMessage("");

    const handlers = {
      playing: () => { failed = false; setStatus("playing"); setMessage(""); },
      pause: () => { if (!audio.ended && !failed) setStatus("paused"); },
      waiting: () => setStatus("loading"),
      ended: () => { setStatus("ended"); if (onEndedRef.current) onEndedRef.current(); },
      error: () => fail(ERROR_TEXT[audio.error && audio.error.code] || "The stream couldn't be played."),
    };
    Object.entries(handlers).forEach(([type, fn]) => audio.addEventListener(type, fn));

    const start = () => {
      if (!autoPlay || cancelled) return;
      // Browsers may block autoplay until the user interacts; the controls still work then
      audio.play().catch((e) => {
        // a broken stream rejects play() too; the error handler has already said why
        if (cancelled || failed || audio.error || (e && e.name === "AbortError")) return;
        setStatus("paused");
        setMessage(e && e.name === "NotAllowedError" ? "Press play to start." : "");
      });
    };

    if (isHls(streamUrl) && !audio.canPlayType("application/vnd.apple.mpegurl")) {
      import("hls.js")
        .then(({ default: Hls }) => {
          if (cancelled) return;
          if (!Hls.isSupported()) { fail("This browser can't play HLS streams."); return; }
          hls = new Hls();
          hls.on(Hls.Events.ERROR, (_e, data) => {
            if (data.fatal) fail("Stream error: " + data.details);
          });
          hls.loadSource(streamUrl);
          hls.attachMedia(audio);
          start();
        })
        .catch(() => { if (!cancelled) fail("HLS streams need the hls.js package (npm install hls.js)."); });
    } else {
      audio.src = streamUrl;
      start();
    }

    return () => {
      cancelled = true;
      Object.entries(handlers).forEach(([type, fn]) => audio.removeEventListener(type, fn));
      if (hls) hls.destroy();
      // Fully stop the old stream so it doesn't keep downloading in the background
      audio.pause();
      audio.removeAttribute("src");
      audio.load();
    };
  }, [streamUrl, payloadError, autoPlay]);

  const statusText = {
    idle: "",
    loading: "Loading...",
    playing: "Playing",
    paused: "Paused",
    ended: "Ended",
    error: "Error",
  }[status];

  return (
    <figure className={className} style={{ margin: 0 }}>
      <figcaption style={{ display: "flex", justifyContent: "space-between", gap: 12 }}>
        <strong style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {payloadError ? "Can't play this stream" : title}
        </strong>
        <span aria-live="polite" style={{ opacity: 0.7 }}>{statusText}</span>
      </figcaption>
      <audio
        ref={audioRef}
        controls
        preload="none"
        aria-label={payloadError ? "Audio player" : title}
        style={{ width: "100%", marginTop: 8 }}
      />
      {message && <p role="alert" style={{ margin: "6px 0 0", color: "#c0392b" }}>{message}</p>}
    </figure>
  );
}
