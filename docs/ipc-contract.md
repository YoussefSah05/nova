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
| `refresh_source` | `{ profileId: string, sourceId: number }` | `void` | 5 |
| `list_channels` | `{ profileId, sourceId, limit, offset, filters? }` | `Channel[]` | 3 |
| `count_channels` | `{ profileId, sourceId, filters? }` | `number` | 3 |
| `list_channel_countries` | `{ profileId, sourceId }` | `string[]` | 3 |
| `list_channel_categories` | `{ profileId, sourceId }` | `string[]` | 3 |
| `list_vod` | `{ profileId, sourceId, limit, offset, filters? }` | `VodItem[]` | 3 |
| `count_vod` | `{ profileId, sourceId, filters? }` | `number` | 3 |
| `list_vod_categories` | `{ profileId, sourceId }` | `string[]` | 3 |
| `list_movies` | `{ profileId, sourceId, limit, offset, filters? }` | `CanonicalMediaSummary[]` (one per canonical entity with an available variant; secret-free) | P07 |
| `count_movies` | `{ profileId, sourceId, filters? }` | `number` | P07 |
| `get_movie` | `{ profileId, entityId }` | `CanonicalMediaDetail` (summary + `variants[]` + entity `resume`) | P07 |
| `set_movie_favorite` | `{ profileId, entityId, isFavorite }` | `void` | P07 |
| `set_movie_in_watchlist` | `{ profileId, entityId, inWatchlist }` | `void` | P07 |
| `resolve_movie_playback` | `{ profileId, entityId, variantId? }` | `MoviePlaybackTarget { entityId, variantId, mediaKind: 'vod', mediaId }` (Rust-selected variant; transient locator) | P07 |
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
| `get_model_status` | `{}` | `{ installed: boolean, modelId: string, availability: 'missing'\|'unverified'\|'available' }` | 6 / M2 |
| `list_internal_models` | `{}` | `{ modelId, capability, backend, availability: 'missing'\|'unverified'\|'available', artifactRevision, observedAt }[]`; never includes an absolute path | M2 |
| `add_external_model_library` | `{}` | `libraryId \| null` (`null` if the native folder picker is cancelled); accepts no path from the webview | M2.3 |
| `list_external_model_libraries` | `{}` | `{ libraryId, displayName, availability: 'available'\|'unavailable', observedAt, models: { modelId, capability, availability: 'missing'\|'unverified'\|'available'\|'unavailable' }[] }[]`; no paths or bookmark data | M2.3 |
| `unload_model` | `{ modelId: string }` | `boolean`: whether a loaded backend was released; `Conflict` while loading, mutating or leased; does not delete artifacts | M2.5 |
| `model_runtime_status` | `{}` | `ModelRuntimeStatus`: `{ loadedModels, activeLeases, busyModels, transitioningModels, estimatedResidentBytes, unknownResidentModels, activePlaybackSessions, queuedWork, activeWorkClass, residentModels, residentModelBudget, queuedJobs, activeJobs, jobSlots, suspendedWork }`; counts/class only, no paths or grants | M2.5 / M2.6 / P06 |
| `get_enrich_model_status` | `{}` | `{ installed: boolean, modelId: string, availability: 'missing'\|'unverified'\|'available' }` | enrichment / M2 |
| `download_whisper_model` | `{}` | `void` (streams `model:download:progress`) | 6 |
| `start_caption_session` | `{ profileId, mediaKind, mediaId, fromSec, durationSec?, audioTrackIndex?, trackLang? }` | `void` (streams `transcription:cues`/`:status`) | 6 |
| `set_caption_playhead` | `{ sec: number }` | `void` | 6 |
| `stop_caption_session` | `{}` | `void` | 6 |
| `get_caption_style` | `{ profileId }` | `CaptionStyle` (defaults if unsaved) | 6 |
| `set_caption_style` | `{ profileId, style: CaptionStyle }` | `void` | 6 |
| `list_player_episodes` | `{ profileId, episodeId }` | `SeriesEpisode[]` (siblings of the episode; `[]` if not an episode) | cleanup |
| `get_profile_settings` | `{ profileId }` | `ProfileSettings` (defaults if unsaved) | cleanup |
| `set_profile_settings` | `{ profileId, settings: ProfileSettings }` | `void` | cleanup |
| `update_profile` | `{ id, name, avatarColor }` | `Profile` | cleanup |
| `delete_source` | `{ profileId, sourceId }` | `void` (cascades imported content) | cleanup |
| `set_source_auto_refresh` | `{ profileId, sourceId, hours }` | `void` | cleanup |
| `remove_whisper_model` | `{}` | `void` | cleanup |
| `get_transcription_cache_stats` | `{ profileId }` | `{ transcripts, bytes }` | cleanup |
| `clear_transcription_cache` | `{ profileId }` | `void` | cleanup |
| `get_artwork_cache_stats` | `{}` | `{ files, bytes }` | artwork v1 — the shared on-disk artwork cache behind the `nova-art://` protocol |
| `clear_artwork_cache` | `{}` | `void` | artwork v1 |
| `list_continue_watching` | `{ profileId, sourceId, section: "movies" \| "series" \| "live", limit? }` | `ContinueWatchingItem[]` | per-section rails: unfinished canonical movies (`entityId`, backdrop art); one card per series (resume the latest episode or `upNext` the following one, `season`/`episode`/`numbering`/`episodeTitle`); recently watched channels (live). Always within one source |
| `get_title_art` | `{ profileId, subject: { kind: "movie", entityId } \| { kind: "series", sourceId, seriesTitle } }` | `{ logoUrl, logoLanguage, logoWidth, logoHeight, backdropUrl }` | artwork v1 — TMDB title logo (metadata language → English → neutral) + text-free backdrop; stored per TMDB work, fetched on demand only when TMDB is enabled, never fails (empty when unknown) |
| `get_title_facts` | `{ profileId, subject: { kind: "movie", entityId } \| { kind: "series", sourceId, seriesTitle } }` | `{ tagline, runtimeMin, rating, ratingSource: "tmdb"\|"provider", certification, trailerUrl, cast: [{ name, character, profileUrl }], directors, studios, status, firstYear, lastYear, seasons: [{ number, name, episodeCount, airDate, posterUrl }], numberOfEpisodes, nextEpisode, collection, plot, genre }` | M4.6 — confident TMDB facts (fetched on demand, stored per work, TTL 7 days airing / 30 otherwise) with provider facts filling gaps; Xtream movies ask `get_vod_info` once (6 s cap). Never fails for TMDB problems |
| `get_season_episodes` | `{ profileId, sourceId, seriesTitle, season }` | `[{ id, season, episode, numbering, title, overview, stillUrl, airDate, runtimeMin, positionSec, durationSec, completed }]` | M4.6 — provider episodes (what plays) merged with TMDB `/tv/{id}/season/{n}` by episode number only when numbering is real, plus provider episode facts and watch progress |
| `get_series_resume` | `{ profileId, sourceId, seriesTitle }` | `ContinueWatchingItem \| null` | where a series continues: the latest unfinished episode, or `upNext` the one after the latest finished episode; `null` when never watched or finished (the page then offers the first episode) |
| `open_trailer` | `{ url }` | `void` | opens a YouTube trailer in the default browser; any other URL is refused (`InvalidInput`) — the webview has no shell permission |
| `match_channel_logos` | `{ profileId, sourceId?: number \| null, forceRefresh?: boolean }` | `boolean` (false: feature off or a pass already running) | artwork v1 — opt-in iptv-org directory; refreshes the on-device index at most weekly, matches logo-less channels in the background, progress on `artwork:channel-logos` |
| `clear_channel_logo_matches` | `{ profileId }` | `void` | artwork v1 — drops the profile's directory logos (feature turned off) |
| `get_logo_directory_status` | `{}` | `{ downloaded, entries, builtAt? }` | artwork v1 |
| `get_app_info` | `{}` | `{ version, dataDir, transcriptsBytes, modelsBytes }` | cleanup |
| `enqueue_translation` | `{ transcript_id: number, target_lang: string, backend: TranslationBackend }` | `JobId` | 7 |
| `get_transcript` | `{ transcript_id: number }` | `Transcript` | 6 |
| `search` | `{ query: string, lang_filter?: string }` | `SearchResult[]` | 8 |
| `update_settings` | `{ key: string, value: unknown }` | `void` | 1 |

### Model runtime (M2.5)

The existing model status commands report `installed: true` when a structurally
usable registered location exists, including an authorized external library.
Availability checks do not load models or download artifacts. Per-library list
commands continue to report each library independently. `available` does not
certify model integrity or successful inference.

`unload_model` releases only the loaded runtime backend. It returns `false` if a
known model is already unloaded, `NotFound` for an unknown logical ID, and
`Conflict` while a load, artifact mutation or active lease prevents unloading.
Downloads and internal Whisper removal use the same exclusion boundary;
`remove_whisper_model` only removes internal artifacts, never external files.

`model_runtime_status` is a nonblocking, best-effort resource snapshot.
`busyModels` counts admitted operations; `transitioningModels` counts entries
whose lifecycle lock is held. Resident estimates exclude unknown sizes, which
are counted in `unknownResidentModels`; zero estimated bytes does not imply no
memory use. Built-in adapters currently report unknown sizes. Whisper's loaded
entry represents a prepared adapter and authorized path; native weights belong
to its per-window subprocess. These commands introduce no frontend model path
or bookmark contract.

The P06 resource governor (`resource_policy.rs`) applies the fixed priority
order: playback, realtime caption/translation, interactive search, explicit
import/refresh, metadata enrichment, background AI/indexing, maintenance
(`activeWorkClass` uses the lowercase class names). Model initialization and
inference use one process-wide model lane while adapter memory costs remain
unknown. Persisted desktop jobs (`add_source`/`refresh_source` imports,
`epg_refresh`, `start_enrichment`) are queued durably and each running job holds
one slot of a bounded job lane (`jobSlots`, shared by all profiles). Selection
is realtime first, then any waiter bypassed eight times (aging), then class,
then FIFO. Active mpv, proxy-stream or AirPlay sessions suspend metadata,
background and maintenance work (`suspendedWork`); realtime, interactive and
explicit work continue. Running jobs yield their slot at cooperative
checkpoints and resume when playback ends. An already-running native call
finishes its bounded unit; CTranslate2/ONNX is never forcibly interrupted.
In-process heavyweight backends are bounded by `residentModelBudget`; idle,
unleased models are evicted least-recently-used first, and a leased model is
never evicted or unloaded. Realtime/interactive work may exceed the budget when
every resident model is leased, so `residentModels` can briefly exceed it.

Import progress may report `stage: "queued"` and enrichment progress
`stage: "queued"` while work waits for resources; enrichment reports
`stage: "cancelled"` when `delete_source` cancels it. Repeated requests for the
same queued work coalesce into one job.

### Canonical movies (P07)

Movies are the first `SourceRecord != CanonicalEntity != StreamVariant` slice.
`vod_items` rows are provider source records, replaced on every refresh. The
import transaction's catalog compiler binds each row to a `StreamVariant` by a
source-scoped, secret-free binding key: a digest of the Xtream stream ID when
known, otherwise of the locator. Each variant belongs to a canonical entity
(`entityId`, opaque), which owns favorite, watchlist, resume and completion.
Identity and user state therefore survive refresh, row-ID reuse and Xtream
credential rotation. An item the provider temporarily drops keeps its entity
and state until it returns. The canonical DTOs carry no stream or source URLs.

The movie browse, detail, favorite/watchlist and play paths use the commands
above. Legacy `list_vod`, `set_vod_*`, `record_watch_progress` and
`get_resume_position` stay for unmigrated surfaces (semantic search and
Continue Watching cards address a row). Movie writes and reads through them are
routed to the owning entity by the storage layer; the per-row columns and
`watch_history` rows are a compatibility mirror. The compiler merges only
records sharing a binding key within one source.

### Entity resolution (P09)

After the compiler, the import transaction runs deterministic entity resolution
across all sources of the profile. Two entities merge only on positive evidence
(a shared provider TMDB ID corroborated by a title or year, or an identical
normalized title *and* year) and never across a hard contradiction (different
media kind, conflicting trusted IDs, different series or episode coordinates,
clearly incompatible years, or a recorded cannot-link). Anything uncertain stays
separate. The oldest entity survives.

`entityId` stays stable for clients, including across merges:

- A merged-away (historical) `entityId` remains valid in every movie command.
  `get_movie`, `set_movie_favorite`, `set_movie_in_watchlist` and
  `resolve_movie_playback` act on the surviving entity. `get_movie` returns the
  survivor's `entityId`, which clients should adopt.
- `list_movies` / `count_movies` return one card per entity per source. A merged
  entity lists every variant in `get_movie.variants`, and the version picker
  plays any of them.
- On merge, favorite/watchlist combine by OR and the most recent resume or
  completion wins. Nothing is dropped: the prior state of both sides is kept in
  the append-only identity lineage.

No new commands or events were added. Model output cannot merge identities.

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
| `artwork:channel-logos` | `{ stage: 'downloading'\|'matching'\|'complete'\|'failed', sourceId?, matched, candidates, done, error? }` | artwork v1 |
| `transcription:cues` | `{ mediaKind, mediaId, language, cues: [{start,end,text}] }` (incremental, absolute times) | 6 |
| `transcription:status` | `{ mediaKind, mediaId, state: 'preparing'\|'streaming'\|'complete'\|'failed', coveredSec?, error? }` | 6 |
| `model:download:progress` | `{ artifact: 'weights'\|'coreml', downloadedBytes, totalBytes? }` | 6 |
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

// Per-profile caption appearance (Phase 6.2). Persisted in the profile DB.
type CaptionStyle = {
  fontScale: number; // 0.85 | 1.0 | 1.25 | 1.5
  fontFamily: "sans" | "serif" | "mono" | "dyslexic";
  textColor: "white" | "yellow" | "cyan";
  background: "shadow" | "outline" | "box";
  bgOpacity: number; // 0..1, used when background === "box"
};

// Per-profile viewing/behaviour settings (cleanup phase). Persisted in the profile DB.
type ProfileSettings = {
  autoplayNext: boolean;
  defaultCaptionMode: string; // "off" | "source" (legacy "translation"/"dual" open as "source")
  preferredAudioLang: string | null;
  preferredSubtitleLang: string | null;
  defaultPlaybackRoute: "auto" | "remux_hls" | "transcode_hls";
};
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
   and calls `airplay_set_picker_rect` to position it. The button is always shown while the mpv
   engine is active (route detection is unreliable as a gate); the picker is positioned whenever the
   controls are visible and no route is engaged.
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

### `refresh_source`

Re-fetches the source's playlist and atomically replaces its content (Replace import mode):
snapshot → clear → re-import → restore, all in one transaction so the browse surfaces never flash
empty. User state is preserved across the swap — favorites/watchlist (matched by `stream_url` /
series title) and movie/live continue-watching (watch history remapped by `stream_url`). Episode
continue-watching cannot be remapped across a re-import and self-drops. Emits the same
`playlist:import:progress` events as `add_source`, scoped to the refreshed `source_id`.

### `playlist:import:progress`

Emitted by `add_source`/`refresh_source`/`run_import` while a playlist is importing.

Stages:
- `connecting`: import worker started and is waiting for the first provider bytes.
- `receiving`: entries are being fetched, parsed, classified, and batch-persisted.
- `persisting`: final buffers and source refresh metadata are being committed.
- `complete`: initial import completed; optional enrichment may continue later.
- `failed`: import ended without usable content or persistence failed.

The UI should stop loading whenever `done` is `true`. A non-null `warning` means the import
completed with recoverable issues, such as one optional provider endpoint failing after other
content was imported. A non-null `error` means the import failed and should be shown to the user.

### Caption session identity (P00 stability correction)

`start_caption_session` requires a frontend-generated `sessionId` string, unique
for each profile/media/audio-track activation. `transcription:cues` and
`transcription:status` echo that `sessionId`. The player subscribes before starting
and ignores events from other sessions. Starts and stops are serialized per engine;
model-download completion cannot restart an inactive session.

## Translation removed (2026-09-27)

The early NLLB/CTranslate2 engine (`nova-translate`, `ct2rs`) was removed: its
statically linked sentencepiece carried a second protobuf copy that crashed ONNX
Runtime when the embedding model loaded. Removed commands: `get_nllb_model_status`,
`download_nllb_model`, `list_target_languages`, `start_translation_session`,
`set_translation_playhead`, `stop_translation_session`; removed events:
`translation:cues`, `translation:status`, `nllb:download:progress`. The player offers
Off / Source captions only. Translation tables and cached rows are kept;
`get_transcript` still returns cached translations when `targetLang` is given, and
transcript search still matches them. The `enqueue_translation` /
`translation:progress` / `translation:complete` rows above describe the planned
Phase 7 contract, not implemented commands. Translation returns in Phase 7/10 after
an engine study (latency, footprint, no conflicting native dependencies).

## TMDB credential (P11)

The stored credential is either TMDB's **API Read Access Token** (preferred; a
JWT sent as a Bearer header) or a legacy v3 API key. The backend detects which.

The credential is **write-only** from the webview:

- `get_profile_settings` never returns it. Instead it returns
  `tmdbCredentialConfigured: boolean` and `tmdbCredentialHint: string | null`
  (the last 4 characters).
- `set_profile_settings` ignores any credential it is sent and keeps the stored
  one.
- `set_tmdb_credential { profileId, credential }` saves or replaces it. A
  malformed value (empty, over 1 KiB, whitespace or control characters) is
  rejected with the new `ApiError` kind `InvalidInput`.
- `clear_tmdb_credential { profileId }` removes it. Enrichment progress
now also emits `catalog:enrich:progress` with `stage: "matching"` after each
committed batch of matches, and clients may refresh on it.

## Metadata language (naming v1)

`ProfileSettings.metadataLanguage` (BCP-47, default `"en-US"`) selects the
language of TMDB titles and overviews. It is read and written through
`get_profile_settings` / `set_profile_settings` like other settings, and
invalid tags fall back to `en-US`.

## Canonical movie search (P12)

`search_movies { profileId, sourceId, query, limit? } -> CanonicalMediaSummary[]`

- It returns canonical entities, one per entity, among those with an available
  variant in `sourceId`.
- Order: exact title first, then title prefix, title words, aliases and
  metadata (genres, keywords, people). The optional semantic leg never outranks
  a title match.
- It replaces `semantic_search_catalog` in the browse search and the Command
  palette. `semantic_search_catalog` still exists but is unused by the UI.
