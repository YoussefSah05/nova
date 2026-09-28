# Nova phase history (to 2026-09-28)

> **Historical record — not current status.** This is the per-phase narrative that used to live in
> the project notes, moved here verbatim on 2026-09-28. It explains *why* past work looks the way it does
> (e.g. whisper `--dtw` without `--vad`, the libmpv `CAOpenGLLayer` embed, the removed `ct2rs`
> engine). For what exists today, trust the repository and
> [`docs/nova-implementation-status.md`](../nova-implementation-status.md); for where Nova is going,
> see [`docs/architecture/long-term-architecture.md`](../architecture/long-term-architecture.md).
> Section references to `ARCHITECTURE.md` below now resolve to
> [`ARCHITECTURE-v2-brief.md`](ARCHITECTURE-v2-brief.md).

## Phase status (as of 2026-09-28)

| Phase | Status |
|---|---|
| **Phase 0 — Scaffolding** | ✅ Complete |
| **Phase 1 — Profiles + storage** | ✅ Complete |
| **Phase 2 — M3U import** | ✅ Complete |
| **Phase 3 — Browse UI** | ✅ Complete |
| **Phase 4 — EPG** | ✅ Complete (4.1 grid guide deferred) |
| **Phase 5 — Playback (standard captions)** | ✅ Complete (Windows/Linux render handlers deferred) |
| Phase 6 — Transcription (VOD) | 🟡 Streaming + sync-tuned captions + style controls implemented (`feat/phase-6-transcription`); on-device verification pending |
| Phase 7 — Translation (VOD) | Not started — the early NLLB/`ct2rs` engine was removed (its sentencepiece protobuf crashed ONNX Runtime); engine to be chosen after a latency/footprint study |
| Phase 8 — Search | Not started |
| Phase 9 — Live transcription | Not started |
| Phase 10 — Live translation | Not started |
| Phase 11 — Cloud backends | Not started |
| Phase 12 — Polish | Not started |
| Phase 13 — Distribution | Not started |

**Phase 5 — Playback is complete and merged to `main`.** Playback was rewritten from the
originally-planned Vidstack/hls.js path to an embedded **libmpv** engine rendered via a
`CAOpenGLLayer` sublayer inside the Nova window (macOS-only so far). Video plays in-window with
synced audio; open/close/switch is crash-free. Player chrome wired: play/pause, scrub, volume/mute,
fullscreen, PiP, seek, live-edge, audio-track / quality (video-track) / speed menus, frame-step
(`,`/`.`), watch-history + resume. AirPlay is implemented as a parallel AVFoundation engine
(`src-tauri/src/airplay.rs`): a native `AVRoutePickerView` overlay + `AVPlayer` with
`allowsExternalPlayback`, fed the proxy HLS URL so hostile IPTV is Apple-TV-playable; mpv pauses
while engaged and resumes on disconnect. The cleanup branch also landed: lazy Xtream
`get_series_info` episode fetch (real seasons/episodes on series open), canonical genre
classification (`crates/core/src/classify.rs`) + a genre chip-bar browse filter, and an atomic
refresh-playlist (re-fetch + reload) that preserves favorites/watchlist/continue-watching.

Deferred follow-up (own branch when picked up): cross-platform render handlers (Windows/Linux).
On-device AirPlay verification (Mac + Apple TV) was confirmed good by the user.

**Phase 6 — Transcription (VOD) is on `feat/phase-6-transcription`.** Approach: `whisper-cli` is
driven as a **subprocess** (Homebrew `whisper-cpp`, Core ML) — same external-binary pattern as
ffmpeg/mpv, not FFI. whisper.cpp has **no `--task` flag** (transcribe is default; passing one prints
help — was an early bug); word timestamps come from `--output-json-full`. The model
(`large-v3-turbo-q5_0`, prebuilt GGML + Core ML, no Python) downloads once via an explicit UI prompt.

**Captions stream (Phase 6.1):** a **caption session** (`apps/desktop/.../caption_session.rs`)
transcribes ~30s windows at the playhead (`extract_audio_window` → `transcribe_wav` →
`shift_transcript`), emits `transcription:cues` incrementally, prefetches ~90s ahead, and **lazily**
covers only what's watched — first caption in seconds, not minutes. Coverage persists in
`transcript_windows` (migration 0008) so re-watch replays instantly. Cues render through the React
`CaptionOverlay` (not mpv native, not the proxy `/captions` path), so Phase 7's translation card
reuses the stream. Commands: `start_caption_session` / `set_caption_playhead` /
`stop_caption_session`, `get_model_status`, `download_whisper_model`; events `transcription:cues` /
`transcription:status`, `model:download:progress`. Backend tests stay hermetic (JSON-fixture parse +
VTT rules), so CI needs no whisper binary/model.

**Sync + styling (Phase 6.2):** transcription now passes `--dtw large.v3.turbo` (DTW-aligned token
timestamps — the accuracy lever) and `--prompt` (previous window's tail) for cross-boundary context.
**No `--vad`:** a smoke test proved whisper.cpp leaves DTW token offsets in the VAD-*compressed*
timeline (segment offsets are remapped, tokens aren't), which would corrupt word timing; instead the
session chunks on whisper's **natural pause-aligned segments**. `caption_session.rs` replaced the
fixed 30s grid with a **segment-aligned sliding cursor** + interval-based coverage: it extracts up to
30s (with 2s overlap), keeps only *complete* segments, and advances the cursor to the last complete
segment's end so a sentence is never hard-cut. Caption appearance is personalizable per profile
(Netflix-style "Aa" panel: size / font / colour / backdrop): `nova_core::CaptionStyle`, migration
0009 `caption_style`, `CaptionStyleRepository`, commands `get_caption_style` / `set_caption_style`,
and `CaptionStylePanel` + `useCaptionStyle` feeding `CaptionOverlay`.

**Stitch fix (Phase 6.3):** long, pauseless discussions dropped lines because the overlap stitch
deleted segments by *start* time — a segment starting inside the lead-in overlap but extending past
the boundary (common in dense speech) was discarded with its new content. Now segments are kept by
where they *end* (past the confirmed frontier `gap_start`) and their already-shown leading words are
trimmed (`trim_leading_words`, LocalAgreement-style) — lossless. Whisper also emits caption-sized
cues via `--max-len 60 --split-on-word` (DTW-compatible).

**Per-track + stable cache (Phase 6.4):** transcription now follows the **selected audio track**
(`extract_audio_window` adds `-map 0:a:<n>`; `start_caption_session` takes `audioTrackIndex` +
`trackLang`; `useSourceCaptions` re-targets + restarts on track change). Transcripts are keyed by
`content_key` (SHA-256 of the stream URL) + `audio_track` + `model` (migration 0010, a safe table
rebuild preserving children), so each track caches independently (switch-back is instant) and a
playlist refresh that renumbers `media_id` no longer orphans the cache (lazy `media_id` re-point +
`content_key` backfill).

**Cleanup phase (`feat/cleanup-series-settings`):** (1) **Cinema-mode episode browser** — the player's
cinema panel shows seasons + episodes for the playing series (current highlighted, click to switch)
via `list_player_episodes`; finishing an episode autoplays the next when enabled. (2) **Settings**
(`features/settings/`, header gear → `SettingsModal`) with four sections: Playback & Captions,
Transcription & Cache, Sources & EPG, Profile & About. Per-profile behaviour settings live in
`profile_settings` (migration 0011, `nova_core::ProfileSettings`); new maintenance commands:
`update_profile`, `delete_source`, `set_source_auto_refresh`, `remove_whisper_model`,
`get_transcription_cache_stats`, `clear_transcription_cache`, `get_app_info`.

**Window controls (`feat/window-controls`):** the `Overlay`-style window had no move-handle. Added
`data-tauri-drag-region` to the top chrome (browse header with a traffic-light inset, profile-picker
strip via `components/DragRegion.tsx`, settings header), the `core:window:allow-start-dragging` +
`allow-internal-toggle-maximize` capabilities, and `tauri-plugin-window-state` to remember
size/position/maximized across launches.

**Window UX fixes (`feat/window-ux`):** (1) the drag handle now reaches **every screen** — `DragRegion`
added to `ContentDetail` (movie + live bundle), the `SeriesDetail` wrapper, and `data-tauri-drag-region`
on the `PlayerModal` top chrome (previously only Home/Settings were draggable). (2) The window is now
**opaque while browsing and transparent only while a video surface is on screen**: `native_player_view`
defaults opaque and exposes `set_window_transparent` + a `set_player_surface_active` command that
`PlayerModal` toggles in its `nativeEmbedded` effect — this kills the bug where fast momentum-scroll
revealed the libmpv `CAOpenGLLayer`'s last frame through un-painted webview tiles. (3) `MpvLayer` is now
`setOpaque(true)` so CoreAnimation stops blending it against the transparent window — fixes the "glassy"
video and the desktop showing through cinema-mode letterbox margins. These are macOS native-compositing
changes (verify with `just dev`).

**Browse performance + modern redesign (`feat/browse-performance`):** audit-driven rework of the
homescreen into a **Netflix-style rails + grid** surface on a **TanStack Query** data layer.
- *Data layer:* `lib/queryClient.ts` (provider in `main.tsx`; 5-min staleTime, no refetch-on-focus) +
  `features/browse/queries.ts` — typed query-key factory + `invoke` fetchers, a normalized `BrowseItem`,
  hooks (`useBrowseCount`/`useCategories`/`useCountries`/`useContinueWatching`/`useRail`/`useBrowseGrid`),
  prefetch (`prefetchSourceOverview`/`prefetchPersonalRails`) and optimistic patch helpers
  (`patchItemEverywhere`, `invalidateMembershipRails`, `invalidateSource`). This replaces the bespoke
  `loadedSigRef`/`totals` cache from the first cut.
- *Surface:* `BrowseRails` = `BrowseHero` (featured item, reuses the Recently-Added rail query) +
  Continue Watching + Watchlist/Favourites/Recently + one `Rail` per genre; `BrowseGrid` (responsive
  `useInfiniteQuery` poster grid, row-windowed via `VirtualRows.tsx`, ResizeObserver columns,
  `keepPreviousData`) when a search/filter/See-all is active. `PosterCard` is the shared card.
  **`Shelves.tsx` deleted** — its per-tab refetch was the tab-switch lag.
- *Why it's fast:* counts+categories+Continue Watching are prefetched for all tabs on source select,
  personal rails on idle; with staleTime, switching tabs is a cache read (no IPC, no skeleton).
- *shadcn/ui adoption + blank-body fix:* the home is now built on **shadcn/ui** primitives
  (`src/components/ui/*`: button/card/badge/input/tabs/skeleton — Radix + cva; deps `class-variance-authority`,
  `@radix-ui/react-slot`, `@radix-ui/react-tabs`), themed dark via shadcn CSS tokens in `globals.css`
  `@theme` (`--color-background/-foreground/-card/-primary/-muted/-border/-ring/...`). `components.json`
  fixed for Tailwind 4 (`config: ""`). The earlier **blank body** (header showed, content void) is
  fixed: the hero + first rails are **eager** (no IntersectionObserver gate on primary content; only
  deep genre rails lazy-load), plus explicit **loading / empty (`count===0`) / error** states and a
  `BrowseBoundary` error boundary so the content area is never empty. eslint override disables
  `react-refresh/only-export-components` for `components/ui/**` (shadcn exports cva fns by convention).
- Kept from the first cut: migration **0012** `idx_channels_source_country`, `loading="lazy"` on
  SeriesDetail images, `.nova-fade-in`. `Home.test.tsx` wraps in `QueryClientProvider`, stubs
  `IntersectionObserver` + `ResizeObserver` + `offsetHeight/Width`, and asserts on role `tab` (Radix Tabs).
  WebKit has no `requestIdleCallback`, so prefetch-on-idle falls back to `setTimeout` (`onIdle` in Home).

**Catalog enrichment — Phase A: rules cleaning (`feat/enrich-rules`):** first layer of a planned
4-layer enrichment pipeline (rules → on-device embeddings (fastembed-rs) → opt-in TMDB → tiny local
LLM residuals) that cleans messy IPTV names + taxonomy for the home. Phase A is the deterministic,
offline, dependency-free rules pass: `nova_core::clean::polish_title` (in `crates/core/src/clean.rs`)
folds decorative Unicode (`ᴴᴰ`→`HD`, fullwidth→ASCII) and strips release/quality/codec junk tokens
(`HD`/`1080p`/`x265`/`WEB-DL`/`MULTI`/`VOSTFR`…), conservatively (real titles like `Spider-Man`,
`Web of Lies` survive; never returns empty). Applied centrally in `push_entry`
(`apps/desktop/src-tauri/src/commands/mod.rs`) to live `name`, vod `clean_title`, and series
`series_title` — so both M3U and Xtream imports benefit with no schema/type change. Existing catalogs
need a **Refresh** (re-import) to re-polish. Phases B–D (embeddings/dedup, TMDB, LLM) will add
enrichment columns + a post-import background job; see the plan.

**Phase B — on-device embeddings (`feat/enrich-embeddings`), backend landed:** new crate
`nova-enrich` (fastembed-rs / `ort` ONNX Runtime, `multilingual-e5-small`): `Embedder` trait +
`FastEmbedder` (model downloaded/cached on first load), `dedup` (bucketed cosine clustering collapses
variant entries), `taxonomy` (zero-shot genre, confidence-gated). Spike confirmed `ort 2.0.0-rc.9`
builds on Apple Silicon. Migration **0013** adds `dedup_group` + `enrich_status` to channels +
vod_items; `ChannelRepository`/`VodRepository` gain `list_unenriched` + `apply_enrichment`. Commands
(`commands/enrich.rs`): `get_enrich_model_status` / `download_enrich_models` (marker-file install,
mirrors whisper) / `start_enrichment` → a background task `run_enrichment` (read unenriched →
spawn_blocking embed+cluster+genre-fill → write back) emitting `catalog:enrich:progress`. Idempotent
(gated on `enrich_status IS NULL`; refresh re-imports → re-runs). All vector logic hermetically tested
(no model in CI). **Frontend wired:** `dedup_group` exposed on the `Channel`/`VodItem` read path and
collapsed per page in `features/browse/queries.ts` (`collapseDuplicates` — one card per group);
genre-fill writes the existing `category`, so it shows in chips/rails automatically; a **Catalog
Enrichment** Settings section (`features/settings/sections/EnrichmentSection.tsx`) downloads the model
+ runs the pass with live progress; Home auto-triggers `start_enrichment` after an import (no-ops
without the model) and invalidates the source's React Query caches on the terminal
`catalog:enrich:progress` event so de-duped cards + genres appear without a manual reload. Genre/dedup
thresholds (0.78 / 0.92) and the per-page collapse are v1, tunable after on-device testing.
(Follow-ups: adopt the durable `JobQueue`; slim fastembed default features for distribution; consider
server-side `GROUP BY dedup_group` for exact cross-page counts.)

**Phase C — opt-in TMDB enrichment (`feat/enrich-tmdb`), landed:** fills a display layer of canonical
titles, posters, backdrops, overviews + real genres for **movies and series** (live is out of scope —
TMDB indexes titles, not feeds). **Offline-first**: nothing hits the network unless the user enters a
TMDB API key (v3 auth) and toggles it on in the **Catalog Enrichment** settings. `nova_enrich::tmdb`
is a thin async `reqwest` client returning raw JSON bodies (so they cache verbatim) + pure
parse/match logic (`parse_search`/`parse_genres`/`best_match` — lexical title-Jaccard + year scoring,
`MIN_CONFIDENCE` 0.6; below floor → `tmdb_status='unmatched'`, a Phase D residual, never a guess).
Migration **0014** adds TMDB columns to `vod_items` (`canonical_title`/`tmdb_id`/`poster_url`/
`backdrop_url`/`overview`/`tmdb_genres`/`tmdb_status`, gated on `tmdb_status IS NULL` independently of
Phase B's `enrich_status`), a `series_meta` sidecar (series have no per-title row — `SeriesSummary`
LEFT JOINs it), and a `tmdb_cache` (key→body) so duplicates + re-runs don't re-hit the API (dedup'd
rows share a cache key → near-zero extra calls). `VodRepository`/`SeriesRepository` gain
`list_for_tmdb`/`apply_tmdb`; `TmdbCacheRepository` get/put. The TMDB stage runs *after* the embedding
stage inside `run_enrichment` (network-bound, on the async runtime not `spawn_blocking`), emitting
`catalog:enrich:progress` `stage:"matching"`. `nova_core::ProfileSettings` gained `tmdb_enabled` +
`tmdb_api_key` (JSON blob, `#[serde(default)]` → no migration). Frontend: `VodItem`/`SeriesSummary`
expose the TMDB fields; `queries.ts` `vodToItem`/`seriesToItem` prefer `canonical_title` then poster;
`BrowseHero` prefers the wide backdrop; `EnrichmentSection` adds a TMDB toggle + key field +
attribution (also in About). Match is lexical v1 (embedding rerank deferred); first-run on a large
catalog makes many sequential calls (25ms-spaced) but the cache makes re-runs cheap.

**Enrichment UX fixes + reliable controls + content overview (`feat/enrich-tmdb`), landed:** an
on-device pass over the above fixed four field-reported issues and added the per-content start screen.
(1) **Enrichment no longer sticks at 0** — `run_enrichment` embeds in 256-row chunks emitting real
`embedding x/total` progress (was one blocking whole-catalog call), adds a `loading-model` stage, and
`start_enrichment` emits a terminal `no-model` event so a manual run can't spin forever;
`EnrichmentSection` renders all states. (2) **Dedup no longer loses content** — grouping is gated on
`nova_core::clean::dedup_key` (strips quality junk, *preserves numbers* so `BeIN Sports 1/2`,
`Rocky/Rocky 2` never merge; cosine is only a secondary guard; `cluster()` is O(n) via a key hashmap),
and `collapseDuplicates` keeps the other members as `variants` on the representative (lossless) with an
"N versions" badge on `PosterCard`. (3) **Better TMDB coverage** — `nova_core::clean::search_title`
builds a clean query (strips provider/country prefixes, brackets, year, `SxxEyy`, junk; year sent
separately); the stage tries movie+year → movie no-year → `/search/multi`, stopping once confident
(`best_scored`); a **weak match (≥0.3) with a poster attaches art only** (keeps the original title) so
nearly everything gets a poster; TMDB genres map through `classify::canonical_genre`. (4) **Reliable
controls on shadcn** — new `components/ui/{switch,slider,tooltip,dropdown-menu}.tsx` (Radix); the
Settings `Toggle` wraps `Switch`; the `PlayerModal` control bar rebuilt on them (`IconButton`+Tooltip,
`ChromeMenu`→`DropdownMenu`, `Scrubber`/`VolumeControl`→`Slider`), behaviour preserved. (5)
**Per-content starting screen** — `features/browse/ContentDetail.tsx`: a backdrop hero with TMDB
overview/genres + **Play**, **Audio** (preferred-language dropdown persisted to `ProfileSettings`,
auto-applied once per stream in `PlayerModal`), **Translate** (disabled "soon"), favourite/watchlist,
and a **version picker** for dedup variants. Movies open it before playback (Home `activate`); live
plays directly; `SeriesDetail` shows the canonical title + overview.

**Live channel grouping (brand + region) + cinema layers (`feat/enrich-tmdb`), landed:** live TV is
now classified into **brand families** and **regions**. `nova_core::clean::channel_brand` folds a
trailing channel number into a brand key (`beIN Sports 1/2/HD` → `bein sports`; protects single-word
numbered brands like `France 24`), and `nova_core::region::region_of` buckets the country code into a
region (MENA/Europe/North America/Latin America/Asia/Africa/Oceania; IPTV `AR`=Arabic→MENA). Migration
**0015** adds `channels.channel_group` + `region` (set at import; existing catalogs need a **Refresh**).
`ChannelRepository` gains a `region` filter on `list`/`count` plus `list_groups` (one bundle per brand,
`GROUP BY COALESCE(NULLIF(channel_group,''), name)`, with `member_count` + representative), `list_in_group`,
and `regions()`; commands `list_channel_groups`/`list_channels_in_group`/`list_channel_regions`
(`ChannelFilters` gains `region`); core `ChannelGroup` type. Frontend: the **live tab fetches bundles**
(`queries.ts` `channelGroupToItem`, `prettyBrand` strips the trailing number), Home adds a **Region**
`FilterSelect` next to Type, `PosterCard` shows an "N channels" badge, and a live bundle (>1 channel)
opens `ContentDetail`'s **channel-list** mode (`list_channels_in_group`) — fixing the old dead-end where
the badge wasn't clickable; singletons play directly. Live **cinema mode** is now two layers:
`CinemaChannelStrip` shows *"More from this group"* (the bundle via `list_channels_in_group`) then
*"More <category>"* (same type, region-preferred via `list_channel_groups`), fed by live context on
`PlayMedia` (`channelGroup`/`category`/`region`). Series TMDB matching uses
`clean::search_series_title` (strips season/series words). Note: the live tab count badge still reflects
total channels, not bundle count (grid paginates by length, so this is cosmetic). Next: Phase D (tiny
local LLM residuals) or Phase 7 (translation).

**Artwork v1 (`fix/artwork-quick-wins` + `feat/artwork-v1`):** one image-URL policy
(`nova_core::artwork::plausible_image_url`: extension-less CDN logos kept, unfilled TMDB templates /
stream links rejected) for every importer; XMLTV `<channel><icon>` logos (`epg_channel_logos`, 0031) and
the opt-in **iptv-org channel directory** (`nova_artwork::directory`, `directory_channel_logos`, 0033 —
downloaded whole ≤ weekly, matched on device by tvg-id then unique name+country; `match_channel_logos`,
Settings → Catalog Enrichment) as channel-logo fallbacks. Precedence: provider → EPG → directory →
placeholder. Every image goes through the **`nova-art://` protocol** backed by `nova_artwork::ArtworkCache`
(magic-byte + size + dimension validation, negative cache, one download per URL, LRU 512 MB in the app
cache dir; Settings shows size + Clear); the UI's `Artwork` component / `useArtworkSrc` tries cached →
original URL → placeholder. **TMDB title logos + text-free backdrops** come from `/images` on demand
(`get_title_art`, `tmdb_title_art` 0032, keyed per TMDB work; logo language: metadata → English →
neutral, never another) and show in the hero + movie detail (`TitleLogo`). Series matches now persist
`series_meta.tmdb_id`.

**Series parity + metadata (`docs/metadata-master-plan` → `feat/library-discovery`, master plan
M4.6 + §14 slices):** Continue Watching is **per tab and per source** (`list_continue_watching` with a
`section`: movies from entity state; series one card per show, resume or **Up next**; live recently
watched). **Provider facts** (`nova_core::ProviderFacts`: plot, cast, director, genre, date, rating,
runtime, YouTube trailer, backdrop) are kept from Xtream lists / `get_series_info` / lazy
`get_vod_info` (migration 0034, `series_provider_facts` holds series facts + the provider's TMDB TV id,
now a trusted id for series matching). **TMDB facts** (`nova_enrich::tmdb_facts`, `get_title_facts`,
`tmdb_title_facts` 0035) are fetched **on demand** per work with TTLs (7 days airing / 30 otherwise;
seasons 7/90) and merged TMDB → provider. **One detail page** (`features/browse/detail/TitleHero`) for
movies and series: meta line, cast, trailer (backend `open_trailer`, YouTube only), series
Resume/Next/Play (`get_series_resume`), season picker + `get_season_episodes` (provider episodes merged
with TMDB season data by number when numbering is real, plus progress). **Discovery**: "More like this",
collections (`get_related_titles`) and "Trending in your library" (`get_trending_in_library`), library
titles only. Deferred: cross-source series identity, user corrections, people pages, in-app trailers.

Phase 4 (EPG) is complete: Xtream `xmltv.php` + M3U `url-tvg` auto-registered, streaming
quick-xml parser, `add_epg_url` for community feeds, and a now/next chip per live channel.
The 4.1 grid guide view at `/guide` is deferred.

Phase 2 import is provider-validated end to end: the large test playlist routes through the
Xtream JSON API and imports 42,212 entries (9,099 live, 27,998 movies, 5,115 series) in about
2.1-2.4s with visible UI progress. Imported content is browseable after completion; Live and
Movie lists load in 200-item pages so large playlists do not block initial rendering.

