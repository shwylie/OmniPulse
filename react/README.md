# StreamPlayer (React)

A standalone React component that plays a `{ title, streamUrl }` payload through an HTML5 `<audio>` element.
It is **not used by OmniPulse itself**, which has no React or build step. Copy `StreamPlayer.jsx`
into any React 18+ app.

```jsx
import StreamPlayer from "./StreamPlayer";

<StreamPlayer payload={{ title: "Live set", streamUrl: "https://example.com/stream.mp3" }} />
<StreamPlayer payload={jsonStringFromServer} autoPlay onError={(e) => console.warn(e)} onEnded={next} />
```

| Prop | |
|---|---|
| `payload` | An object or a JSON string with `title` (optional) and `streamUrl` (required, `http`/`https` only; relative URLs are resolved against the page). |
| `autoPlay` | Start as soon as the stream is set. If the browser blocks autoplay, it says "Press play to start." |
| `onError(error)` | Bad payload, unreachable or undecodable stream, or a fatal HLS error. |
| `onEnded()` | The stream finished. |
| `className` | Put on the outer `<figure>`. |

- Plain streams (MP3, AAC, Ogg, WAV, Icecast) play natively.
- HLS (`.m3u8`) plays natively where the browser supports it, and otherwise through
  [hls.js](https://github.com/video-dev/hls.js), loaded on demand only if installed (`npm install hls.js`).
- Changing `payload` stops the old stream completely (no background download) and loads the new one.
- `parsePayload(payload)` is exported too, if you want to validate before rendering.
