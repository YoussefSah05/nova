# IPC Contract

This document specifies the Tauri command and event API between the Rust backend and the React frontend.
TypeScript bindings are generated from Rust types via `ts-rs` and placed in `apps/desktop/src/types/`.

> **Status**: Phase 2 import and Phase 3 imported-content browse commands are implemented.

---

## Commands (request/response)

| Command | Payload | Response | Phase |
|---|---|---|---|
| `list_profiles` | `{}` | `Profile[]` | 1 |
| `create_profile` | `{ name: string, avatarColor: string }` | `Profile` | 1 |
| `delete_profile` | `{ id: string }` | `void` | 1 |
| `select_profile` | `{ id: string }` | `Profile` | 1 |
| `list_sources` | `{ profileId: string }` | `Source[]` | 2 |
| `add_source` | `{ profileId: string, name: string, url: string }` | `number` source id | 2 |
| `list_channels` | `{ profileId, sourceId, limit, offset, filters? }` | `Channel[]` | 3 |
| `count_channels` | `{ profileId, sourceId, filters? }` | `number` | 3 |
| `list_channel_countries` | `{ profileId, sourceId }` | `string[]` | 3 |
| `list_channel_categories` | `{ profileId, sourceId }` | `string[]` | 3 |
| `list_vod` | `{ profileId, sourceId, limit, offset, filters? }` | `VodItem[]` | 3 |
| `count_vod` | `{ profileId, sourceId, filters? }` | `number` | 3 |
| `list_vod_categories` | `{ profileId, sourceId }` | `string[]` | 3 |
| `list_series` | `{ profileId, sourceId, filters? }` | `SeriesSummary[]` | 3 |
| `count_series` | `{ profileId, sourceId, filters? }` | `number` | 3 |
| `list_series_categories` | `{ profileId, sourceId }` | `string[]` | 3 |
| `list_series_episodes` | `{ profileId, sourceId, seriesTitle }` | `SeriesEpisode[]` | 3 |
| `resolve_stream` | `{ profileId, mediaKind: 'live'\|'vod'\|'episode', mediaId, routeOverride?: 'remux_hls'\|'transcode_hls' }` | `ResolvedStream` | 5 |
| `stop_playback` | `{ token: string }` | `void` | 5 |
| `mpv_open_stream` | `{ profileId, mediaKind, mediaId, title }` | `{ token, engine: 'mpv', embedded }` | 5 |
| `mpv_close` | `{ token }` | `void` | 5 |
| `mpv_player_state` | `{ token? }` | `MpvPlayerState` | 5 |
| `mpv_toggle_pause` | `{ token }` | `void` | 5 |
| `mpv_seek_relative` | `{ token, deltaSec }` | `void` | 5 |
| `mpv_seek_absolute` | `{ token, positionSec }` | `void` | 5 |
| `mpv_seek_live_edge` | `{ token }` | `void` | 5 |
| `mpv_set_volume` | `{ token, volume }` | `void` | 5 |
| `mpv_set_mute` | `{ token, muted }` | `void` | 5 |
| `mpv_set_audio_track` | `{ token, id }` | `void` | 5 |
| `mpv_set_subtitle_track` | `{ token, id }` | `void` | 5 |
| `mpv_set_video_track` | `{ token, id }` | `void` | 5 |
| `mpv_set_speed` | `{ token, speed }` | `void` | 5 |
| `mpv_frame_step` | `{ token, forward }` | `void` | 5 |
| `airplay_set_picker_rect` | `{ rect: { x, y, width, height } \| null }` | `void` | 5 |
| `airplay_engage` | `{ profileId, mediaKind, mediaId, positionSec }` | `{ proxyToken }` | 5 |
| `airplay_disengage` | `{ proxyToken? }` | `void` | 5 |
| `airplay_toggle_pause` | `{}` | `void` | 5 |
| `airplay_seek_absolute` | `{ positionSec }` | `void` | 5 |
| `airplay_set_volume` | `{ volume }` | `void` | 5 |
| `airplay_set_mute` | `{ muted }` | `void` | 5 |
| `airplay_status` | `{}` | `AirPlayStatus` | 5 |
| `record_watch_progress` | `{ profileId, mediaKind, mediaId, positionSec, durationSec?, completed, captionMode? }` | `void` | 5 |
| `get_resume_position` | `{ profileId, mediaKind, mediaId }` | `ResumeInfo \| null` | 5 |
| `get_proxy_port` | `{}` | `number` (deprecated - use `resolve_stream`) | 5 |
| `enqueue_transcription` | `{ media_kind: MediaKind, media_id: number, lang?: string }` | `JobId` | 6 |
| `enqueue_translation` | `{ transcript_id: number, target_lang: string, backend: TranslationBackend }` | `JobId` | 7 |
| `get_transcript` | `{ transcript_id: number }` | `Transcript` | 6 |
| `search` | `{ query: string, lang_filter?: string }` | `SearchResult[]` | 8 |
| `update_settings` | `{ key: string, value: unknown }` | `void` | 1 |

Browse filter payloads use camelCase keys from the frontend:

```ts
type ChannelFilters = {
  country?: string | null;
  category?: string | null;
  query?: string | null;
};

type ContentFilters = {
  category?: string | null;
  query?: string | null;
};
```

Use `limit` and `offset` for Live and VOD pagination. The Home UI currently requests 200 rows
per page and uses the corresponding `count_*` command to decide whether to show **Load more**.
`list_series` returns grouped series summaries; use `list_series_episodes` for episode detail.

## Events (backend -> frontend)

Event names use colon namespaces because Tauri rejects dots in event names.

| Event | Payload | Phase |
|---|---|---|
| `playlist:import:progress` | `{ sourceId, stage, parsed, inserted, liveCount, movieCount, seriesCount, failedCount, skippedCount, elapsedMs, done, error?, warning? }` | 2 |
| `transcription:progress` | `{ job_id: number, pct: number }` | 6 |
| `transcription:complete` | `{ job_id: number, transcript_id: number }` | 6 |
| `translation:progress` | `{ job_id: number, pct: number }` | 7 |
| `translation:complete` | `{ job_id: number, transcript_id: number, target_lang: string }` | 7 |
| `live:caption` | `{ channel_id: number, cue: VttCue }` | 9 |
| `live:caption:translated` | `{ channel_id: number, target_lang: string, cue: VttCue }` | 10 |
| `jobs:queue:changed` | `{ jobs: JobSummary[] }` | 2 |
| `epg:refresh:progress` | `{ source_id: number, pct: number }` | 4 |
| `mpv:state` | `MpvPlayerState` | 5 |
| `airplay:status` | `AirPlayStatus` (poll while a route is reachable / engaged) | 5 |
| `airplay:picker-closed` | `void` (user finished with the native route picker) | 5 |

## Playback (Phase 5)

Nova now defaults to the native `libmpv` engine for viewing. The Rust backend
looks the upstream URL up from the profile DB and opens it in libmpv; React owns
controls, watch history, and the caption overlay. The older WebKit/hls.js
`resolve_stream` path remains as a fallback and for proxy/transcription work.
On macOS, Nova installs a `CAOpenGLLayer` subclass as a sublayer of the
WKWebView's own layer tree and drives it through libmpv's render API
(`vo=libmpv`). Because the layer lives inside the webview's layer tree, AppKit
handles z-order, live resize, fullscreen, and PiP transitions natively — no
sibling-view layout fight. Native playback requires this render surface; if it
is unavailable, `mpv_open_stream` returns an in-app error instead of opening a
standalone mpv window.

```ts
type PlaybackKind = "hls" | "file";
type PlaybackRoute = "hls_passthrough" | "file_passthrough" | "remux_hls" | "transcode_hls";
type CaptionAssetState = "unavailable" | "pending" | "ready";
type MpvPlayerPhase = "opening" | "buffering" | "playing" | "paused" | "ended" | "error";

type MpvOpenResult = {
  token: string;
  engine: "mpv";
  embedded: boolean;
};

type MpvTrack = {
  id: number;
  /// "audio" | "video" | "sub"
  kind: string;
  title: string | null;
  lang: string | null;
  selected: boolean;
};

type MpvPlayerState = {
  token: string | null;
  engine: "mpv";
  phase: MpvPlayerPhase;
  positionSec: number;
  durationSec: number | null;
  volume: number;
  muted: boolean;
  seekable: boolean;
  /// Playback rate (1.0 = normal). Set via `mpv_set_speed`.
  speed: number;
  /// Currently selected track ids, or null when none/disabled. The
  /// `mpv_set_*_track` commands take the id as a string ("no" disables).
  audioTrackId: number | null;
  subTrackId: number | null;
  videoTrackId: number | null;
  /// Audio/video/sub tracks reported by libmpv for the current file.
  tracks: MpvTrack[];
  error: string | null;
};

type ResolvedStream = {
  /// URL to give the player. For kind === 'hls' it points at
  /// http://127.0.0.1:<port>/stream/<token>/master.m3u8 served by the proxy.
  /// For kind === 'file' it's the proxy's /proxy?url=... passthrough with
  /// byte-range support, suitable for the native <video> element.
  url: string;
  kind: PlaybackKind;
  /// Backend route decision. MPEG-TS transport streams are not classified as
  /// HLS unless the upstream is an actual playlist.
  route: PlaybackRoute;
  /// Opaque proxy session token. Pass back to `stop_playback` on unmount so
  /// the proxy can release any ffmpeg remux subprocess.
  token: string;
  supportsSeek: boolean;
  durationSec: number | null;
  sourceLang: string | null;
  container: string | null;
  videoCodec: string | null;
  audioCodec: string | null;
  width: number | null;
  height: number | null;
  /// Non-null when playback is using fast fallback after probe/remux failure.
  fallbackReason: string | null;
  /// Phase 5 returns unavailable; Phase 6/7 upgrade these without changing
  /// the playback session contract.
  captionState: {
    source: CaptionAssetState;
    translation: CaptionAssetState;
  };
};
```

Player flow:

1. `invoke("mpv_open_stream", { profileId, mediaKind, mediaId, title })` opens native playback.
2. Subscribe to `mpv:state` events for time, duration, volume, mute, and phase updates. The
   backend emits these whenever libmpv signals new events (driven by a wakeup callback). Call
   `mpv_player_state` once on mount for the initial snapshot, and keep a long-interval (~2.5s)
   poll as a fallback in case an event is dropped.
3. Use `mpv_*` control commands for play/pause, seeking, volume, mute, live edge, audio/video/
   subtitle track selection, playback speed, and frame-step. `MpvPlayerState.tracks` drives the
   audio-track and quality (video-track) menus; menus show only when ≥2 tracks of that kind exist.
4. Every 5s while playing, `invoke("record_watch_progress", { ..., captionMode: "off" })`.
5. On modal close / unmount: `invoke("mpv_close", { token })`.

AirPlay (parallel AVPlayer route, macOS):

1. The backend installs a native `AVRoutePickerView` overlay; the frontend reserves a button slot
   and calls `airplay_set_picker_rect` to position it. The button shows only while a route is
   reachable (`airplay:status.available`).
2. When the user picks a device, `airplay:picker-closed` fires. The frontend calls `airplay_engage`,
   which resolves the proxy HLS URL (so hostile IPTV becomes Apple-TV-friendly) and starts a muted
   `AVPlayer`. Once `airplay:status.engaged` confirms the external route activated, the frontend
   pauses mpv and unmutes via `airplay_set_mute`; if it never engages within ~6s it reverts.
3. While engaged, transport uses the `airplay_*` commands and state comes from `airplay:status`.
4. On disconnect (engaged → not engaged) or Stop, the frontend resumes mpv at the AirPlay position
   and calls `airplay_disengage` (releasing the proxy session). Switching media drops the AirPlay
   session; the new content plays locally.

Fallback WebKit flow:

1. `invoke("resolve_stream", { profileId, mediaKind, mediaId })` -> `ResolvedStream`.
2. If `kind === "hls"`: attach `hls.js` to `url`. Otherwise set `<video src={url}>`.
3. If the web player raises a decode/src-not-supported error, stop the current session and retry
   `resolve_stream` with `routeOverride: "remux_hls"`, then `"transcode_hls"` if remuxed output
   still cannot decode.
4. On modal close / unmount: `invoke("stop_playback", { token })`.

### Proxy routes (Phase 5)

| Route | Phase | Purpose |
|---|---|---|
| `GET /health` | 5 | Liveness check. Returns `"ok"`. |
| `GET /proxy?url={encoded}` | 5 | Proxies HLS sub-playlists and segments after manifest rewriting, and direct file playback for `kind === "file"`. Rewrites `.m3u8` responses recursively and forwards `Range` for media bytes. |
| `GET /stream/{token}/master.m3u8` | 5 | Manifest endpoint. Behaviour depends on session route: passthrough-HLS rewrites the upstream manifest; remuxed/transcoded sessions serve the file ffmpeg is writing. |
| `GET /stream/{token}/local/{file}` | 5 | Serves files from the remux session's temp dir (segments). |
| `GET /captions/{token}/{lang}.vtt` | 6 | Source-language captions. Returns 404 in Phase 5. |
| `GET /captions/{token}/{src}-to-{tgt}.vtt` | 7 | Translated captions. Returns 404 in Phase 5. |

### `playlist:import:progress`

Emitted by `add_source`/`run_import` while a playlist is importing.

Stages:
- `connecting`: import worker started and is waiting for the first provider bytes.
- `receiving`: entries are being fetched, parsed, classified, and batch-persisted.
- `persisting`: final buffers and source refresh metadata are being committed.
- `complete`: initial import completed; optional enrichment may continue later.
- `failed`: import ended without usable content or persistence failed.

The UI should stop loading whenever `done` is `true`. A non-null `warning` means the import
completed with recoverable issues, such as one optional provider endpoint failing after other
content was imported. A non-null `error` means the import failed and should be shown to the user.
