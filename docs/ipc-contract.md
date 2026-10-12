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
| `inspect_profile_recovery` | `{ profileId: string }` | `ProfileRecoveryStatus` (`available` / `unsupported`); read-only recognition of the exact supported legacy lineage, no profile activation | M26 |
| `recover_profile_copy` | `{ profileId: string }` | `Profile`; converts into a new private profile directory and registers only after validation, keeping the original unchanged; does not select or start jobs | M26 |
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
| `list_movies` | `{ profileId, sourceId, limit, offset, filters? }` — filters add `genre` (Nova's primary genre), `yearFrom`/`yearTo` (inclusive), `collectionId` (TMDB); `sort`: `default`/`recentlyAdded`/`title`/`year`/`rating` | `CanonicalMediaSummary[]` (one per canonical entity with an available variant; secret-free; card facts `genres`, `runtimeMin`, `rating` + `ratingSource`, `badges` (best quality + audio versions across the title's sources), `versionCount`, all from stored evidence) | P07, UI-2 |
| `list_movie_genres` | `{ profileId, sourceId }` | `{ name, count }[]` — Nova's genres, most common first | UI-2 |
| `list_movie_decades` | `{ profileId, sourceId }` | `{ decade, count, posterUrls }[]` — newest first, up to 4 posters | UI-2 |
| `list_library_collections` | `{ profileId, sourceId }` | `{ id, name, count, posterUrls }[]` — TMDB collections with 2+ films in the playlist, known from fetched facts | UI-2 |
| `count_movies` | `{ profileId, sourceId, filters? }` | `number` | P07 |
| `list_library_history` | `{ profileId, sourceId, kind?: 'vod'\|'episode'\|'live', limit?, offset? }` | `LibraryHistoryItem[]` — latest watch per canonical film, durable episode or current channel in the playlist; includes completed and unavailable items (`mediaId: null`), newest first with a stable key tie-break. Default limit 51, clamped 1–100; offset non-negative. No stream URLs. Generated DTO: `just bindings`. | UI-6 |
| `get_movie` | `{ profileId, entityId }` | `CanonicalMediaDetail` (summary + `variants[]` with `badges`, `recentlyFailed` and `picked` — the variant selector's first choice for the profile's audio language — + entity `resume`, `originalTitle`, `releaseDate`, `voteCount`, `keywords`) | P07, UI-2 |
| `set_movie_favorite` | `{ profileId, entityId, isFavorite }` | `void` | P07 |
| `set_movie_in_watchlist` | `{ profileId, entityId, inWatchlist }` | `void` | P07 |
| `resolve_movie_playback` | `{ profileId, entityId, variantId? }` | `MoviePlaybackTarget { entityId, variantId, mediaKind: 'vod', mediaId }` (Rust-selected variant: explicit choice, no recent failure, preferred audio language, picture quality; transient locator) | P07, M14 |
| `list_series` | `{ profileId, sourceId, filters? }` | `SeriesSummary[]` | 3 |
| `count_series` | `{ profileId, sourceId, filters? }` | `number` | 3 |
| `list_series_categories` | `{ profileId, sourceId }` | `string[]` | 3 |
| `list_series_genres` | `{ profileId, sourceId }` | `{ name, count }[]` — Nova's genres of the playlist's series, most common first | UI-3 |
| `list_series_schedule` | `{ profileId, sourceId, from, to }` | `SeriesSummary[]` — series whose next TMDB-scheduled episode airs between the inclusive local `YYYY-MM-DD` dates, soonest first (at most 60); only series whose facts were fetched are known; `InvalidInput` for other date forms | UI-3 |
| `list_series_cards` | `{ profileId, sourceId, titles }` | `SeriesSummary[]` — fresh cards for up to 50 series titles, any order | UI-3 |
| `list_series_episodes` | `{ profileId, seriesId }` | `SeriesEpisode[]` | 3 |
| `resolve_stream` | `{ profileId, mediaKind: 'live'\|'vod'\|'episode', mediaId, routeOverride?: 'remux_hls'\|'transcode_hls' }` | `ResolvedStream` | 5 |
| `stop_playback` | `{ token: string }` | `void` | 5 |
| `epg_now_and_next` | `{ profileId, tvgIds: string[], sourceId?: number }` | `NowAndNext[]` (one per requested id; a channel shows one feed's guide — with `sourceId`, that source's first feed covering it, else the first covering feed — so now and next never mix feeds) | 4, M24 |
| `epg_range` | `{ profileId, tvgIds, fromUtc, toUtc, sourceId?: number }` | `Program[]` (each channel from the same single guide feed as `epg_now_and_next`; no duplicates across feeds); `InvalidInput` beyond 200 ids or a 13-hour window | 4, M24 |
| `epg_guide` | `{ profileId, channelIds: number[], fromUtc, toUtc }` | `ChannelGuide[] { channelId, programs: Program[] }` — the Live guide grid: one per requested id, in order, each channel's programmes overlapping `[fromUtc, toUtc)` from its own source's feed where it has one (empty without a guide); one query per request; `InvalidInput` beyond 200 ids or a 13-hour window | UI-4 |
| `list_live_overview` | `{ profileId, sourceId }` | `{ channels, withGuide, categories: { name, count }[] }` — channel count, how many have guide data, channels per category | UI-4 |
| `epg_refresh_if_stale` | `{ profileId, sourceId }` | `boolean` — queues `epg_refresh` when any of the source's feeds was never refreshed or not in 12 hours; the Live section asks once per session | UI-4 |
| `add_epg_url` | `{ profileId, sourceId, url }` | `void` (registers a community feed and queues the source's guide refresh; a failure to queue is logged, the feed stays saved) | 4, M24 |
| `epg_channels_now_and_next` | `{ profileId, channelIds: number[] }` | `ChannelNowAndNext[] { channelId, now, next }` (one per requested id, in order; each channel reads its own source's guide feed; empty for channels without a `tvgId`) — what cards, previews and the live player use, batched client-side and read in one query per call | M24 |
| `list_epg_feeds` | `{ profileId, sourceId }` | `EpgFeedStatus[] { id, kind, lastRefreshUtc, lastError, programmeCount }` — never the feed URL (Xtream feed URLs carry credentials); URLs in stored errors are redacted | M24 |
| `epg_refresh` | `{ profileId, sourceId }` | `void` (durable job: each feed is downloaded to a temp file capped at 512 MiB (gzip feeds are inflated while parsing, up to 2 GiB), parsed into bounded staged batches, then replaces only its own programmes in one short transaction; a feed that parses to no programmes fails and keeps its previous guide). Also queued automatically after every successful import (add or refresh) | 4, M24 |
| `mpv_open_stream` | `{ profileId, mediaKind, mediaId, title, startSec? }` — optional finite, non-negative dialogue timestamp overrides saved resume for this opening (one-second lead-in); ignored for live | `{ token, engine: 'mpv', embedded }` | 5, UI-5 |
| `mpv_close` | `{ token }` | `void` | 5 |
| `playback_caption_position` | `{ token }` | `number \| null` (actual native clock once the coordinator has acknowledged resume, or bounded seek retries have ended; no URLs) | Caption startup |
| `playback_caption_mode` | `{ token, captionMode }` | `void` | M14 — the caption selection, persisted with the native session's progress by the playback coordinator |
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
| `record_watch_progress` | `{ profileId, mediaKind, mediaId, positionSec, durationSec?, completed, captionMode? }` | `void` | 5 — web (hls.js) fallback path only; native progress is written by the playback coordinator |
| `get_resume_position` | `{ profileId, mediaKind, mediaId }` | `ResumeInfo \| null` | 5 |
| `get_proxy_port` | `{}` | `number` (deprecated - use `resolve_stream`) | 5 |
| `get_model_status` | `{}` | `{ installed: boolean, modelId: string, availability: 'missing'\|'unverified'\|'available' }` | 6 / M2 |
| `list_internal_models` | `{}` | `{ modelId, capability, backend, availability: 'missing'\|'unverified'\|'available', artifactRevision, observedAt }[]`; never includes an absolute path | M2 |
| `list_model_controls` | `{}` | Generated `LocalModelStatus[]`: `{ modelId, capability, availability, library: 'internal'\|'external', enabled, activity: 'idle'\|'loading'\|'prepared'\|'loaded'\|'inUse' }`; effective runtime location precedence, discovery only | Model controls |
| `set_model_enabled` | `{ modelId: string, enabled: boolean }` | `void`; persistent app-wide permission, idle-only change, disable unloads and blocks new acquisitions, enable never eagerly loads/downloads | Model controls |
| `add_external_model_library` | `{}` | `libraryId \| null` (`null` if the native folder picker is cancelled); accepts no path from the webview | M2.3 |
| `list_external_model_libraries` | `{}` | `{ libraryId, displayName, availability: 'available'\|'unavailable', observedAt, models: { modelId, capability, availability: 'missing'\|'unverified'\|'available'\|'unavailable' }[] }[]`; no paths or bookmark data | M2.3 |
| `unload_model` | `{ modelId: string }` | `boolean`: whether a loaded backend was released; `Conflict` while loading, mutating or leased; does not delete artifacts | M2.5 |
| `model_runtime_status` | `{}` | `ModelRuntimeStatus`: `{ loadedModels, activeLeases, busyModels, transitioningModels, estimatedResidentBytes, unknownResidentModels, activePlaybackSessions, queuedWork, activeWorkClass, residentModels, residentModelBudget, queuedJobs, activeJobs, jobSlots, suspendedWork }`; counts/class only, no paths or grants | M2.5 / M2.6 / P06 |
| `get_enrich_model_status` | `{}` | `{ installed: boolean, modelId: string, availability: 'missing'\|'unverified'\|'available' }` | enrichment / M2 |
| `download_whisper_model` | `{}` | `void` (streams `model:download:progress`) | 6 |
| `start_caption_session` | `{ sessionId, profileId, mediaKind, mediaId, fromSec, durationSec?, audioTrackIndex?, trackLang? }` | `void` (streams `transcription:cues`/`:status`) | 6 |
| `get_high_refresh_rate` | `{}` | `boolean` (app-wide; default `true`) | Display |
| `set_high_refresh_rate` | `{ enabled: boolean }` | `void` (persists and re-applies WebKit's 60 fps pacing on the main webview at once) | Display |
| `download_translation_models` | `{}` | `void` (streams `model:download:progress`; installs every unverified pinned translation model, checksum-verified, then attested) | M18 |
| `translate_caption_segments` | `{ profileId, sessionId, transcriptId, segmentIds: number[], targetLang: string }` | `CaptionCue[]` (`segmentId,start,end,text`) | M18 |
| `set_caption_playhead` | `{ sec: number }` | `void` | 6 |
| `stop_caption_session` | `{}` | `void` | 6 |
| `get_caption_style` | `{ profileId }` | `CaptionStyle` (defaults if unsaved) | 6 |
| `set_caption_style` | `{ profileId, style: CaptionStyle }` | `void` | 6 |
| `list_player_episodes` | `{ profileId, episodeId }` | `SeriesEpisode[]` (siblings of the episode; `[]` if not an episode) | cleanup |
| `get_profile_settings` | `{ profileId }` | `ProfileSettings` (defaults if unsaved) | cleanup |
| `set_profile_settings` | `{ profileId, settings: ProfileSettings }` | `void` | cleanup |
| `update_profile` | `{ id, name, avatarColor }` | `Profile` | cleanup |
| `delete_source` | `{ profileId, sourceId }` | `void` (cascades imported content) | cleanup |
| `set_source_auto_refresh` | `{ profileId, sourceId, hours }` | `void`; `hours` must be `0` (manual, the default) or `6`, `12`, `24`, `168`, else `InvalidInput`. A due playlist refreshes automatically as background work while its profile is active | cleanup, M16 |
| `remove_whisper_model` | `{}` | `void` | cleanup |
| `get_transcription_cache_stats` | `{ profileId }` | `{ transcripts, bytes }` | cleanup |
| `clear_transcription_cache` | `{ profileId }` | `void` | cleanup |
| `get_artwork_cache_stats` | `{}` | `{ files, bytes }` | artwork v1 — the shared on-disk artwork cache behind the `nova-art://` protocol |
| `clear_artwork_cache` | `{}` | `void` | artwork v1 |
| `list_continue_watching` | `{ profileId, sourceId, section: "movies" \| "series" \| "live", limit? }` | `ContinueWatchingItem[]` | per-section rails: unfinished canonical movies (`entityId`, backdrop art); one card per series (resume the latest episode or `upNext` the following one, `season`/`episode`/`numbering`/`episodeTitle`); recently watched channels (live). Always within one source |
| `get_title_art` | `{ profileId, subject: { kind: "movie", entityId } \| { kind: "series", seriesId } }` | `{ logoUrl, logoLanguage, logoWidth, logoHeight, backdropUrl }` | artwork v1 — TMDB title logo (metadata language → English → neutral) + text-free backdrop; stored per TMDB work, fetched on demand only when TMDB is enabled, never fails (empty when unknown) |
| `get_title_facts` | `{ profileId, subject: { kind: "movie", entityId } \| { kind: "series", seriesId } }` | `{ tagline, runtimeMin, rating, ratingSource: "tmdb"\|"provider", certification, trailerUrl, cast: [{ name, character, profileUrl }], directors, studios, status, firstYear, lastYear, seasons: [{ number, name, episodeCount, airDate, posterUrl }], numberOfEpisodes, nextEpisode, collection, plot, genre }` | M4.6 — confident TMDB facts (fetched on demand, stored per work, TTL 7 days airing / 30 otherwise) with provider facts filling gaps; Xtream movies ask `get_vod_info` once (6 s cap). Never fails for TMDB problems |
| `get_season_episodes` | `{ profileId, seriesId, season }` | `[{ id, season, episode, numbering, title, overview, stillUrl, airDate, runtimeMin, positionSec, durationSec, completed }]` | M4.6 — provider episodes (what plays) merged with TMDB `/tv/{id}/season/{n}` by episode number only when numbering is real, plus provider episode facts and watch progress |
| `get_series_resume` | `{ profileId, seriesId }` | `ContinueWatchingItem \| null` | where a series continues: the latest unfinished episode, or `upNext` the one after the latest finished episode; `null` when never watched or finished (the page then offers the first episode) |
| `open_trailer` | `{ url }` | `void` | opens a YouTube trailer in the default browser; any other URL is refused (`InvalidInput`) — the webview has no shell permission |
| `get_related_titles` | `{ profileId, sourceId, subject }` | `{ collectionName, collection: CanonicalMediaSummary[], movies: CanonicalMediaSummary[], series: SeriesSummary[] }` | M4.6 T2 — TMDB recommendations (similar as fallback) and the movie's collection, **only titles the library has in `sourceId`**, in TMDB order; cached 30 days; empty with TMDB off |
| `get_trending_in_library` | `{ profileId, sourceId, section: "movies" \| "series" }` | `{ movies, series }` | M4.6 T3 — this week's TMDB trending (2 pages) intersected with the library; cached a day |
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
Series filters (`list_series`, `count_series`): `category`, `query`, `favoritesOnly`, `watchlistOnly`,
`genre` (Nova's primary genre) and `status` (`"airing"`: a scheduled episode or a returning /
in-production TMDB status; `"ended"`: ended or cancelled). `sort`: `recentlyAdded`, `title`,
`year` (TMDB first air year) or `rating`; anything else keeps the title order.
Summaries carry card facts from stored evidence and the profile's progress (UI-3): `genres`,
`rating` + `ratingSource` (`tmdb` | `provider`, zero is unrated), `status` (TMDB's, verbatim),
`firstYear`, `lastYear`, `nextEpisode { season, episode, name, airDate }`, `watchedCount`
(finished numbered episodes) and `lastWatchedAt`.
Series are addressed by `seriesId` — the canonical series entity (`SeriesSummary.entityId`) —
never by provider title: `list_series_episodes`, `get_season_episodes`, `get_series_resume`,
`set_series_favorite { profileId, seriesId, isFavorite }`,
`set_series_in_watchlist { profileId, seriesId, inWatchlist }` and the `series` art/facts/related
subject. Rust resolves where the series is listed now, so a reference made before a provider
rename stays valid; an entity that isn't a listed series is `NotFound`.

## Events (backend -> frontend)

Event names use colon namespaces because Tauri rejects dots in event names.

| Event | Payload | Phase |
|---|---|---|
| `playlist:import:progress` | `{ sourceId, stage, parsed, inserted, liveCount, movieCount, seriesCount, failedCount, skippedCount, elapsedMs, done, error?, warning? }` | 2 |
| `playlist:import:background` | `playlist:import:progress` payload plus `profileId` (source ids are per profile), for a scheduled (automatic) refresh. The UI picks up its catalog on success and never shows its progress or errors | M16 |
| `artwork:channel-logos` | `{ stage: 'downloading'\|'matching'\|'complete'\|'failed', sourceId?, matched, candidates, done, error? }` | artwork v1 |
| `transcription:cues` | `{ sessionId, mediaKind, mediaId, transcriptId, language, cues: [{segmentId,start,end,text}] }` (committed, incremental, absolute times) | 6/M18 |
| `transcription:status` | Generated `CaptionStatusEvent`: `{ sessionId, mediaKind, mediaId, state: 'preparing'\|'streaming'\|'complete'\|'failed', phase: string\|null, coveredSec: number\|null, error: string\|null }`; phase reports preparation/extraction/model waiting/recognition, null clears active work | 6 / Model controls |
| `model:download:progress` | `{ artifact: 'weights'\|'coreml'\|<translation model id>, downloadedBytes, totalBytes? }` (translation totals span all models being installed) | 6, M18 |
| `translation:progress` | `{ job_id: number, pct: number }` | 7 |
| `translation:complete` | `{ job_id: number, transcript_id: number, target_lang: string }` | 7 |
| `live:caption` | `{ channel_id: number, cue: VttCue }` | 9 |
| `live:caption:translated` | `{ channel_id: number, target_lang: string, cue: VttCue }` | 10 |
| `jobs:queue:changed` | `{ jobs: JobSummary[] }` | 2 |
| `epg:refresh:progress` | `{ sourceId, epgSourceId, stage: 'downloading'\|'parsing'\|'complete'\|'failed', inserted, elapsedMs, error }` (per feed) | 4, M24 |
| `epg:refresh:done` | `{ sourceId, ok }` — a source's guide refresh job ended, however it ended (cancelled, failed, done); invalidates cached guides | M24 |
| `mpv:state` | `MpvPlayerState` | 5 |
| `playback:ended` | `{ token, nextEpisode: SeriesEpisode \| null, autoplay }` — native media played to its end; next episode and autoplay decided in Rust | M14 |
| `playback:recover` | `{ token, mediaKind, mediaId, attempt, otherSource, reason: 'failure'\|'wake' }` — `failure`: the native session failed; bounded recovery (same item once, then another source of the title, ≤ 3 attempts) decided in Rust. `wake`: the Mac woke while a live channel played; reconnect it (attempt 0, outside the recovery budget). The player reopens it for its own token | M15, M29 |
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
  /// For kind === 'file' it's http://127.0.0.1:<port>/stream/<token>/file,
  /// with byte-range support, suitable for the native <video> element.
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
2. Subscribe to `mpv:state` events for time, duration, volume, mute, and phase updates. A newly
   loaded file reports `opening` until libmpv has loaded it, and `ended` when it plays to its end
   (`keep-open` pauses on the last frame, so the end is read from `eof-reached`). The
   backend emits these whenever libmpv signals new events (driven by a wakeup callback). Call
   `mpv_player_state` once on mount for the initial snapshot, and keep a long-interval (~2.5s)
   poll as a fallback in case an event is dropped.
3. Use `mpv_*` control commands for play/pause, seeking, volume, mute, live edge, audio/video/
   subtitle track selection, playback speed, and frame-step. `MpvPlayerState.tracks` drives the
   audio-track and quality (video-track) menus; menus show only when ≥2 tracks of that kind exist.
4. Playback policy is Rust-owned (`PlaybackCoordinator`): from the engine's own snapshots it
   seeks to the resume point once playable, persists progress every 5 s while playing, on pause,
   at the end, on close and before another item replaces the session, and decides completion
   (≥ 95 %). The player only reports the caption selection:
   `invoke("playback_caption_mode", { token, captionMode })`. `record_watch_progress` /
   `get_resume_position` remain for the web (hls.js) fallback path.
5. On a playback error the coordinator records the source's failure (`variant_health`) and may
   emit `playback:recover { token, mediaKind, mediaId, attempt, otherSource }`: the player reopens
   that item (same id: reopen; other id: open it) for its own token. Reaching playback records a
   success. Movies pick their source with the variant selector (explicit choice, no recent failure,
   preferred audio language, picture quality).
6. When the media ends, the backend emits `playback:ended { token, nextEpisode, autoplay }`; the
   player opens `nextEpisode` only for its own current token and when `autoplay` is set.
7. On modal close / unmount: `invoke("mpv_close", { token })` (the final write happens here).

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

### Proxy routes (Phase 5, M25)

Every media route is scoped to a session token (created by `resolve_stream` / `airplay_engage`,
released by `stop_playback` / `airplay_disengage` / a page reload). No upstream URL appears in any
locator, and the proxy fetches only URLs the session's own manifests named, never on this machine
(literal loopback hosts, and names resolving only to loopback, are refused): there is no open relay.
`/proxy?url=` was removed. A playlist is recognised by URL, content type or its `#EXTM3U` start, and
its relative URIs resolve against the post-redirect URL.

| Route | Phase | Purpose |
|---|---|---|
| `GET /health` | 5 | Liveness check. Returns `"ok"`. |
| `GET /stream/{token}/master.m3u8` | 5 | Manifest endpoint. Behaviour depends on session route: passthrough-HLS rewrites the upstream manifest; remuxed/transcoded sessions serve the file ffmpeg is writing. |
| `GET /stream/{token}/r/{id}` | M25 | A segment, key or sub-playlist the session's manifests named, by opaque per-session id; sub-playlists are rewritten the same way. |
| `GET /stream/{token}/file` | M25 | A direct-file session's bytes (`kind === "file"`), `Range` forwarded. |
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

## App lifecycle (M29)

No command: Rust owns it (`lifecycle.rs`).

- **Webview page load** (a reload, and the first load): the page epoch moves on at once. Playback
  opened for an older page is released — the native session after its progress write (bounded to
  2 s), AirPlay, proxy sessions and their ffmpeg remuxes, caption transcription — with its
  guards, so background work resumes. What the new page already opened stays. An open still in
  flight for the old page (`mpv_open_stream`, `resolve_stream`, `airplay_engage`) undoes itself and
  fails with "the page reloaded".
- **App exit**: the native session's progress is written (bounded), proxy sessions, remuxes and
  transcription stop, every guard drops; mpv and AirPlay end with the process.
- **Sleep** (`NSWorkspaceWillSleepNotification`): the native session pauses, which writes its
  progress. **Wake**: after 3 s for the network, a live channel that was playing gets
  `playback:recover { reason: 'wake' }` (with a fresh recovery budget) and reconnects at the live
  edge, unless the Mac slept again first; VOD and a channel the user had paused stay paused.

## Legacy translation removed (2026-09-27)

The early NLLB/CTranslate2 engine (`nova-translate`, `ct2rs`) was removed: its
statically linked sentencepiece carried a second protobuf copy that crashed ONNX
Runtime when the embedding model loaded. Removed commands: `get_nllb_model_status`,
`download_nllb_model`, `list_target_languages`, `start_translation_session`,
`set_translation_playhead`, `stop_translation_session`; removed events:
`translation:cues`, `translation:status`, `nllb:download:progress`. Translation tables and cached rows are kept;
`get_transcript` still returns cached translations when `targetLang` is given, and
transcript search still matches them. The `enqueue_translation` /
`translation:progress` / `translation:complete` rows above describe the planned
Phase 7 contract, not implemented commands. The new M18 path below uses the evaluated
Opus-MT engine on the app's single ONNX Runtime.

## Committed caption translation (M18, 2026-10-09)

`translate_caption_segments` accepts one to eight distinct positive segment IDs.
The session, profile and transcript must match the active caption session. Text
and timestamps are read from storage; arbitrary frontend text cannot be submitted.
Only one batch may run per session; a busy or replaced session returns `Conflict`.
Track/media changes, closing the player and turning captions off cancel model work
through the caption session. Target/mode changes discard obsolete frontend results.

Targets and source dialogue support English, French and Arabic, with BCP-47/ISO
track-tag normalization. The manifest selects direct models or an English pivot.
Every hop acquires a runtime lease and Realtime resource admission independently.
The command never downloads models or invokes a network translation provider.

Cache lookup precedes inference. `transcript_translations.backend` holds a JSON
provenance key (`pipeline`, `decoding`, `source`, `target`, `sourceHash`, and ordered
`models: [{id,revision}]`). Legacy keys remain readable through legacy APIs but are
not reused by this path. Output cardinality/nonempty text is validated before one
transaction writes the batch. Clearing transcripts still cascades translations.

The player offers Off / Source / Translation / Source + translation, plus local
English/French/Arabic targets. Missing/offline models and inference failure preserve
source captions and show a safe status with a retry action. `C` cycles the four
modes for VOD/episodes. Live AI captions remain unavailable pending audio capture.

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

## Detail audio languages

`get_audio_languages { profileId, mediaKind, mediaId } -> AudioLanguages`

- Resolves one playable `vod`, `episode` or `live` row privately in its profile database.
- Returns deduplicated uppercase audio language tags (`languages`), `unknownTracks` and
  `trackCount`, plus ordered `tracks: { audioIndex: number, language: string | null }[]`.
  `audioIndex` is the zero-based **audio-only** stream ordinal (`0:a:<index>`), not
  the container stream index; duplicate-language and untagged tracks are retained.
  ISO-639-2 tags and regional tags are normalized to compact codes where
  known. Subtitle tags and a title's original language are excluded.
- Uses a six-second audio-only ffprobe within an eight-second admission/probe budget;
  shares the bounded interactive job lane. No playback starts and no URLs, stream titles,
  paths or raw probe output cross IPC. Missing/failed probes return a safe structured error.
- Requested and cached only for active detail targets. Movie details resolve Nova's selected
  version through `resolve_movie_playback`; series describes only its next/resume episode.
  Missing tags mean unknown, not unavailable audio. This command does not install media tools.
  Media tools preserve explicit environment overrides, search PATH first, then standard macOS
  Homebrew locations so desktop launches can use already installed FFmpeg/ffprobe.

## Per-play audio and caption startup

Detail Audio menus show only probe-confirmed tracks in the selected source. Auto is the
native/profile selection; an explicit track overrides it for this playback only. Choosing
another version clears the transient track choice. Unknown or duplicate-language tracks
include their one-based display track number. Probe loading/failure offers status/retry,
without constructing language choices from titles, subtitles or profile preferences.

Frontend playback accepts optional `PlaybackLaunchOptions` with `captionMode`,
`translationTarget` (EN/FR/AR), and a source-bound `{ mediaKind, mediaId, index, language }`
audio request. Shared Play/Resume controls launch off/source/translation explicitly.
Existing callers retain their profile behavior. No profile writes or database migration.
Every explicit launch creates a new player request, including reopening the same title.
Automatic next episode retains the viewer's current caption mode/target and discards the
previous episode's track ordinal; each new source resolves its own audio tracks.

Video opens immediately. Native captions wait for both actual audio selection acknowledgement
and `playback_caption_position` (polling only during startup). The backend observes a fresh
native snapshot on its ordered coordinator before returning the settled clock. Unavailable
tracks or failed/three-second unconfirmed switches visibly fall back to automatic/current
audio. Disabled models point to Settings → Models and Retry; missing models retain explicit
download actions. Translation failures continue to render source captions. Live AI captions
remain unavailable.

## Moments: dialogue across the library (UI-5)

`search_moments { profileId, sourceId, query, limit? } -> MomentHit[]`

- Searches the dialogue of every transcribed film and episode of the playlist: FTS5 over
  spoken and cached translated text, best match first, one hit per line, at most 50. Lexical
  only; the per-title `search_transcript` stays hybrid.
- Fewer than two Unicode characters after trimming returns no hits. Candidates are scoped
  to the playlist before the 400-match ranking cap. Matches for the same media row in the
  same rounded half-second bucket are collapsed across tracks/models/translations before
  applying the result limit.
- A hit carries the line (`text`, `kind`, `startSec`, `endSec`) and where it was said:
  `mediaKind` (`vod` | `episode`) + `mediaId` to play, `title`, `season`/`episode`/
  `episodeTitle` with `numbered`, `posterUrl` and `entityId`.
- Live transcripts are excluded (no replayable timeline).
- The Search page plays a hit with `PlayMedia.startSec`. Native opening forwards it to the
  playback coordinator, which owns readiness/retries and overrides saved resume with a
  one-second lead-in (clamped to zero). Recovery resumes the current session's progress,
  rather than restarting the original moment. The web fallback also skips saved resume
  when an explicit moment is provided.
- Playlist publication reattaches orphaned film/episode transcripts by their stream URL's
  content key, within the publication transaction. This requires the URL to stay the same;
  it is not canonical transcript identity across URL changes.
