# Nova Architecture & Player 2.0 Technical Plan

> **Superseded (2026-09-28) — historical reference only.** Nova's long-term architecture is
> [`docs/architecture/long-term-architecture.md`](../architecture/long-term-architecture.md);
> current implementation status is [`docs/nova-implementation-status.md`](../nova-implementation-status.md).
> Do not use this document for new design decisions. It is kept because code comments, ADRs,
> commits and the status document cite its sections and milestone IDs.

Status: proposed implementation plan; no runtime changes are implied by this document.

Baseline: repository commit `c2841bc`, inspected 17 September 2026. External platform references were checked on the same date. Performance budgets below are proposed acceptance targets unless explicitly identified as existing measurements.

## 1. Executive architecture decision

Harden Nova incrementally as a macOS-first, local-first modular application: Tauri 2 hosts React/TypeScript presentation, Rust owns playback and background work, SQLite stores each profile's durable data, and embedded libmpv remains the primary playback engine. Retain the existing import, browse, search, transcription, translation, and enrichment implementations behind clearer interfaces.

The central addition is a Rust `PlaybackCoordinator` managing one foreground `PlaybackSession`. It owns stream resolution, typed Live/Movie/Episode policies, recovery, the playback clock, output handoffs, and cancellation. React sends user intents and renders authoritative snapshots; it does not run a second playback state machine.

Build reproducibility and a deterministic macOS test harness come before major player changes. The first delivery milestone is a validated Apple Silicon alpha, not completion of every premium feature. Timeshift, DVR, additional platforms, and a general-purpose local LLM are separate investments.

Five decisions correct assumptions in the brainstorming and existing documentation:

1. **Keep the current OpenGL embedding initially.** Nova currently selects `vo=libmpv` and uses the OpenGL render API through `CAOpenGLLayer`. GPU rendering, `gpu-next`, libplacebo, Metal, and HDR are not interchangeable capabilities. A future renderer change requires its own compatibility spike against the exact pinned mpv build. The upstream manual distinguishes these output paths. [mpv manual](https://mpv.io/manual/stable/)
2. **Translation currently uses CPU inference.** `ct2rs` explicitly selects `Device::CPU`; “CTranslate2/Metal” in older docs is inaccurate. Keep this working backend while measuring actual latency. [CTranslate2 hardware support](https://opennmt.net/CTranslate2/hardware_support.html)
3. **Native mpv PiP is currently disabled.** `PlayerModal.tsx` explicitly sets PiP support to false when the mpv engine is active; its remaining PiP controls use HTML video APIs. Earlier status summaries overstated this capability. Expose output capabilities honestly and validate a native adapter separately.
4. **NLLB distribution is a product gate.** The upstream model is CC-BY-NC-4.0, and its model card describes research use rather than production deployment. Do not assume the converted INT8 weights remove those restrictions. Resolve intended distribution rights or select a validated, appropriately licensed replacement behind the existing translator interface. [NLLB model card](https://huggingface.co/facebook/nllb-200-distilled-600M)
5. **macOS E2E is feasible but has two layers.** Current Tauri guidance supports WebdriverIO through an embedded test driver on macOS. It does not replace native video, system-window, AirPlay, or accessibility validation. [Tauri WebDriver guidance](https://v2.tauri.app/develop/tests/webdriver/)

## 2. Architecture principles

1. Preserve working behavior; migrate one boundary at a time.
2. Use known, pinned, validated versions; upgrade through evidence, not automatic freshness.
3. One authority per fact: Rust for playback, repositories for persistent data, native lockfiles for resolved packages, generated reports for the consolidated stack.
4. Keep the catalog usable without models, TMDB, or successful enrichment.
5. Treat media identity, session identity, provider credentials, and transport URLs as different concepts.
6. Scope every asynchronous result to a profile, session, and generation; discard stale work.
7. Playback and audio continuity have priority over AI, indexing, artwork, and thumbnails.
8. Cap queues, memory, retries, subprocess lifetimes, caches, and disk usage.
9. Capabilities determine controls; unsupported features do not masquerade as working ones.
10. Require measured regression evidence for “complete.” Mocked UI tests do not establish native correctness.
11. Keep provider inputs untrusted and diagnostics private by default.
12. Keep one monorepo and a small number of cohesive modules; no service fleet or universal media abstraction.

## 3. Current → target architecture

| Area | Keep | Refactor/add |
|---|---|---|
| Desktop | Tauri, React, existing browse UI and query layer | Thin commands, generated contracts, player presentation store |
| Playback | `crates/mpv-player`, macOS render surface | Coordinator, session lifecycle, typed policies, capability reporting |
| Transport | M3U/Xtream, existing HLS proxy and FFmpeg utilities | Central resolver, credential references, bounded recovery; choose direct/proxy routes explicitly |
| AirPlay | Existing AVFoundation adapter | Move handoff authority from React into Rust; verify reachability and format support |
| Captions | Whisper subprocess, existing stitching and caches, NLLB translator | Unified timeline, revision-aware cache keys, output-specific rendering |
| Catalog | Per-profile SQLite, deterministic cleanup, enrichment and search | Stable content identities, generation-based invalidation, durable cancellable jobs |
| Environment | Cargo/pnpm workspaces, `just`, lockfiles | Exact pins, verified native/model manifests, doctor, release provenance |
| Tests | Rust and Vitest tests | Real macOS smoke, fixture provider, UI Lab, visual tests, hardware qualification |

Concrete baseline issues found in code:

- `rust-toolchain.toml` uses `stable`; mise uses `stable`, major-only Node/pnpm, and `just = "latest"`. CI also installs stable Rust and Homebrew mpv.
- `crates/mpv-player/build.rs` discovers system mpv through `pkg-config` or Homebrew library paths. The installed native graph is not pinned by Cargo.lock.
- Model downloads use mutable Hugging Face `resolve/main` paths; file existence is currently treated as installation evidence.
- `AppState::catalog_vectors` keys its cache by source ID and kind, and invalidates using row count. Target keys must include profile, model and data generation; equal row counts do not mean equal data. This is a correctness risk to test immediately, not a confirmed reproduction.
- Storage enables `set_ignore_missing(true)` for migrations. Ignoring unknown migrations is not proof of safe downgrade compatibility.
- Tauri currently has `csp: null`; production content policy needs explicit design and verification.
- Documentation mixes current and obsolete pipelines. README and agent instructions must stop serving as independent status databases.

Do not replace Tauri with Electron, rebuild the app in SwiftUI, make HTML video/hls.js the main engine, write a raw FFmpeg player, split repositories, or add a server database. Retain the browser fallback only for explicitly tested capabilities during migration; remove unused paths after parity rather than maintaining ambiguous automatic fallbacks forever.

## 4. System architecture diagram

```text
                         Nova macOS application
  +------------------------------------------------------------------+
  | React / WKWebView                                                 |
  | Browse + PlayerChrome + Panels + CaptionLayer + Transcript         |
  | presentation state only; generated command/event contracts        |
  +-------------------------+----------------------------------------+
                            | intents / snapshots / clock anchors
  +-------------------------v----------------------------------------+
  | Tauri commands -> Rust PlaybackCoordinator                       |
  | PlaybackSession / state machine / cancellation / output handoff   |
  |                                                                  |
  | LivePolicy      MoviePolicy      EpisodePolicy                    |
  |      |               |                |                          |
  |      +---------------+----------------+                          |
  |                      v                                           |
  | StreamResolver -> resolved transport lease -> EngineAdapter       |
  | credentials        direct or local proxy          |               |
  | refresh/retry                                 libmpv              |
  |                                                   |              |
  |                         FFmpeg/libavformat demux + networking      |
  |                                                   |              |
  |                       VideoToolbox decode / software fallback     |
  |                                                   |              |
  |                         mpv GPU rendering -> macOS surface         |
  |                  current: OpenGL; future renderer: qualified ADR  |
  +------------------------------------------------------------------+
       | clock/track events              | AppKit/native bridge
       v                                 +-> NSWindow/fullscreen
  CaptionTimeline                       +-> media keys/Now Playing
       ^                                 +-> display/sleep/wake
       |                                 +-> AirPlay output adapter
  embedded/external adapters             +-> native PiP capability
  Whisper -> source cues -> translator -> translated cues
       |                               |
       +---- per-profile SQLite -------+
                  |
             FTS + embeddings

  ImportCoordinator -> M3U/Xtream -> atomic catalog generation
                                        |
                         cleanup + optional embeddings/TMDB
  EPGService -> XMLTV -> channel-scoped schedules -> Live UI
```

```text
Optional future live capture path (not required for initial alpha):

Provider -> CaptureSession -> bounded rolling SegmentStore
                                  |            |
                                  |            +-> RecordingService/DVR
                                  +-> local playback manifest -> mpv
                                  +-> PCM tap -> live ASR -> CaptionTimeline

Discontinuity epochs + media timestamps connect video, audio, EPG,
captions and DVR. A single shared capture avoids redundant provider pulls.
```

Logical modules remain in the Tauri process initially. FFmpeg and whisper-cli are managed subprocesses. WebKit uses its platform-managed processes. Heavy in-process inference runs off the UI/async executor threads; process isolation for CT2/ONNX is a later option if crash or memory evidence warrants it.

## 5. Domain model

The following sketches define boundaries, not drop-in compilable code. Serialize dedicated DTOs rather than native handles, secrets, or Rust `Instant` values.

```rust
enum PlaybackKind {
    Live(LivePlaybackOptions),
    Movie(VodPlaybackOptions),
    Episode(EpisodePlaybackOptions),
}

enum PlaybackState {
    Idle,
    Resolving,
    Connecting,
    Buffering { reason: BufferReason },
    Playing,
    Paused,
    Seeking { target: MediaTime },
    Reconnecting { attempt: u32, retry_at: MonotonicTime },
    Ended,
    Failed(PlaybackFailure),
}

struct PlaybackSession {
    id: SessionId,
    generation: u64,
    profile_id: ProfileId,
    media: MediaIdentity,
    kind: PlaybackKind,
    state: PlaybackState,
    desired_transport: PlayOrPause,
    output: OutputRoute,
    clock: PlaybackClock,
    tracks: Vec<Track>,
    capabilities: PlaybackCapabilities,
    preferences: PlaybackPreferences,
    recovery: RecoveryPolicy,
}

struct StreamRequest {
    media: MediaIdentity,
    provider: ProviderId,
    locator: ProviderLocator,
    credential_ref: Option<CredentialRef>,
    kind: PlaybackKind,
    transport_hint: Option<ContainerHint>,
}

// Private backend type; never sent to React or ordinary diagnostics.
struct ResolvedStream {
    lease: TransportLeaseId,
    endpoint: SensitiveUrl,
    headers: SensitiveHeaders,
    expires_at: Option<UtcTime>,
    route: DirectOrProxy,
    refresh: Option<RefreshHandle>,
}

struct Track {
    key: TrackKey,
    engine_id: EngineTrackId,
    kind: TrackKind,
    language: Option<LanguageTag>,
    role: TrackRole, // main, commentary, descriptive, forced, etc.
    codec: Option<String>,
    channels: Option<u16>,
    selected: bool,
}

enum CaptionSource {
    Embedded(TrackKey),
    External(SubtitleAssetId),
    Generated { transcript: TranscriptId, model: ModelFingerprint },
    Translated { parent: CaptionRevision, model: ModelFingerprint },
    Live { capture: CaptureSessionId, epoch: TimelineEpoch },
}

enum LiveEdge {
    Unknown,
    Known {
        epoch: TimelineEpoch,
        seekable_start: MediaTime,
        playable_end: MediaTime,
        presentation_target: MediaTime,
        observed_at: MonotonicTime,
    },
}

struct PlaybackPreferences {
    audio_languages: Vec<LanguageTag>,
    caption_selection: CaptionSelection,
    caption_style: CaptionStyle,
    speed: PlaybackRate,
    live_buffer_mode: LiveBufferMode,
    autoplay_next: bool,
}

struct RecoveryPolicy {
    maximum_attempts: u32,
    total_deadline: Duration,
    backoff: BackoffWithJitter,
    preserve_position: bool,
    allow_url_refresh: bool,
    restore_live_target: bool,
}

struct QoEMetrics {
    startup: StartupMilestones,
    stalls: StallCounters,
    seek: SeekMeasurements,
    decoder: DecoderSnapshot,
    live_distance: Option<Duration>,
    reconnect_count: u32,
}
```

Use integer media time units internally, explicit UTC only for schedule/wall-clock mappings, and monotonic time for durations. Avoid mixing seconds-from-start, MPEG timestamps and wall time in plain numbers.

`MediaIdentity` is profile/source/provider scoped and separate from its changing URL. Prefer durable provider IDs plus a content revision; use cautious fingerprints when IDs are unavailable. Never collapse different edits, audio versions or episodes merely because titles match.

The coordinator is an actor with a bounded command queue. Exactly one owner issues engine commands. A stream switch increments generation, cancels old resolution/AI work, persists the old position, closes output resources, and starts the replacement. Callbacks only enqueue events; they never perform UI work while holding engine locks.

Commands carry request IDs and session/generation guards. Absolute controls such as `SetPaused(true)` are safe to retry; toggles are translated at the coordinator. Snapshot sequence numbers let React discard old events and request a fresh snapshot after gaps.

Transitions preserve intent: buffering while the user is paused must not resume playback; seeking during recovery replaces the pending target; an intentional stop or switch must not trigger reconnect or episode autoplay. Profile switch and app shutdown cancel work before closing databases. An engine recovery is distinct from an application crash.

## 6. Frontend architecture

```text
NovaPlayer
  VideoSurfaceHost
  PlayerChrome
    TopChrome
    CenterHUD
    BottomChrome
      TransportControls
      VodTimeline | LiveStatus/LiveTimeline
  CaptionLayer
  PlayerPanels
    LanguagesAndCaptionsPanel
    EpisodesPanel
    ChannelsAndEpgPanel
    PlaybackSettingsPanel
    DiagnosticsPanel
  ToastAndHUDLayer
  ModeSpecificUI
    LiveUI | MovieUI | EpisodeUI
```

Extract the existing large `PlayerModal.tsx` incrementally. Keep routing, selected content and integration behavior stable while moving one panel/controller at a time. Reuse Radix/headless primitives, tokens, existing Motion and React Query. Do not introduce a second component suite.

Rust owns state, tracks, capabilities, errors, duration, clock, live edge, recovery and preferences that affect playback. A small frontend store owns open panel, focus, hover, control visibility, drag preview and animation state. React Query continues to own catalog and settings queries; it is not a playback clock.

Event flow: user intent → typed Tauri command → coordinator → engine → authoritative event/snapshot → subscribed UI selectors. Subscribe before requesting the initial snapshot and reconcile using sequence numbers. Use semantic events plus approximately 4–10 Hz clock anchors; a lightweight animation loop may interpolate progress only while playing, with bounded extrapolation. Pause, seek, speed change, buffering and output handoff reset the anchor. Suspend work when the view is hidden.

Caption activation can use the same anchored clock with bounded drift; do not stream 60 Hz component-tree updates. The backend remains the time authority.

Design tokens define spacing, control sizes, typography, contrast, radii, material opacity and transition duration. Use system UI typography for controls; retain Nova's catalog identity where appropriate. Use CSS for simple fades and Motion for layout transitions. Do not introduce experimental React features as a foundation; View Transitions are optional and capability-gated.

Accessibility is structural: semantic buttons/sliders, named panels, focus restoration, Escape hierarchy, keyboard operation and understandable time values. Native appearance/accessibility preferences feed one theme store. Reduced motion removes movement; reduced transparency replaces blur with opaque materials; increased contrast strengthens boundaries. Do not hide focused controls or announce every clock/caption update to VoiceOver.

## 7. Live TV architecture

Use a `LivePolicy` with capability-aware tuning, bounded recovery and a measured live target. “Fast / Balanced / Stable” are policy presets, not exposed numeric buffer settings. Start Balanced; change buffering with hysteresis and measured stalls rather than oscillating after every packet fluctuation.

Resolver responsibilities include authentication, provider URL refresh, redirects, header propagation and choosing direct versus proxy playback. Direct mpv playback remains the default when it works. Use the proxy for a demonstrated transport/output requirement, shared capture or timeshift; do not remux every stream unconditionally.

Centralize retry ownership. Classify authentication, expired locator, temporary transport error, malformed content and unsupported codec separately. Re-resolve once where appropriate; use bounded backoff and a total deadline. Avoid multiplying resolver, proxy and engine retries into a long invisible stall.

“Go Live” means seek to the latest safe playable target, usually behind the absolute end by a policy margin. A live edge is known only from actual transport/capture evidence. EPG programme end is not the live edge. For unseekable MPEG-TS, expose Live and reconnect, not a fictional seek range or precise delay.

Channel selection is immediate UI feedback; Enter commits tuning, Escape cancels browsing. Debounce or cancel superseded tune requests. Show channel identity and Now/Next while connecting. Avoid a spinner for very short transitions; around 300 ms show subtle tuning feedback, and use an explicit state for prolonged delays. Never silently display the previous channel as if it were the new one.

EPG keys include source/provider and channel ID. Refresh and parse off the playback path, normalize UTC, tolerate missing data, and preserve explicit local overrides. A stale schedule never prevents watching.

Timeshift is a separate capture capability, not a promise based on mpv cache. Introduce a disk-bounded, discontinuity-aware `SegmentStore`; publish local playlists from committed segments. Track playable coverage, not just file timestamps. Keep segments pinned while readers or recording finalizers use them, and recover indexes after crashes.

DVR promotes segments into recordings with durable metadata and quotas. Scheduled recording must report sleep, unavailable network, expired credentials, conflicts and disk exhaustion; a closed laptop cannot be assumed to record. Provider connection limits govern recording, previews and concurrent playback.

Live ASR should consume a shared capture/audio tap, not start another upstream subscription. VAD may be added only with a verified mapping back to original media timestamps. Bound caption backlog; when inference falls behind, show delayed/unavailable status or reduce work rather than accumulating indefinitely.

## 8. Movie architecture

Keep a single VOD policy for seeking, resume, tracks, speed and captions. Resolve stream capabilities before promising exact seeking. During dragging, show a local preview; coalesce preview work and commit the intended seek on release. Completion is observed when the engine reaches the target and resumes the intended play/pause state, not when IPC returns.

Persist resume periodically, on pause/switch, and at orderly shutdown; tolerate a bounded loss after crashes. Store completion separately from position and define near-end behavior explicitly. A failed network retry preserves position, selected tracks, speed and pause intent.

Generate thumbnail sprites asynchronously with the pinned FFmpeg build. Key them by content revision, video track, output geometry and generator version; cap CPU, disk and extraction concurrency. Show time/transcript text immediately when thumbnails are missing. Imported chapters and trusted intro/credits markers remain distinguishable from inferred markers.

Reuse playback-anchored Whisper windows and overlap stitching. Resolve audio via stable track metadata and verified mpv-to-FFmpeg stream mapping; a numerical track index alone is not portable across engines, remuxes and provider variants. Seek and audio changes cancel obsolete work. Limit prefetch and share network policy with playback.

NLLB remains the development translation adapter until distribution and quality gates are resolved. AI model failure must not stop the movie. Preferences resolve in this order: current explicit selection → content override → series override when applicable → source-language rule → profile default → engine default.

HDR must be treated as an end-to-end capability: decode format, renderer, drawable precision, transfer function, display and output route. Establish correct SDR and HDR-to-SDR behavior first. Qualify HDR separately on physical supported displays; decoding HEVC Main10 alone does not prove HDR presentation.

## 9. Series architecture

Represent episode playback with stable series/season/episode identity and an ordered queue. Reuse VOD transport and AI behavior; add episode context rather than a second engine.

Autoplay requires a confirmed natural end from the active session, an enabled preference and an available next episode. Start a cancellable countdown, expose Cancel and Play Now, and ensure duplicate end events cannot advance twice. Stop, error, seek-to-end and output handoff are not interchangeable with natural completion.

Preload the next episode's metadata and resolve its locator near the end. Avoid a second active media connection until provider limits and stream expiry make prefetch safe. Cancel preloads when the queue or profile changes. Next-episode failures leave the current episode recoverable.

The episode panel stays in-player, keeps current selection visible and preserves focus. Series language preferences use language/role metadata, not stale engine track IDs.

Skip Intro/Recap/Credits is enabled only when trusted or user-defined marker intervals exist. Automatic detection is optional future work with confidence, provenance and correction controls. Do not infer marker quality from the presence of a skip button.

## 10. Caption architecture

Unify clock, selection, metadata, provenance and timing; do not require every subtitle format to pass through a lossy text conversion.

`CaptionTimeline` stores cues with source/track identity, epoch, start/end time, revision, language, text or an asset reference, optional word timings and provisional/final status. Embedded text, external SRT/VTT, AI source cues and translated cues enter through adapters. Translation references a particular source revision so superseded text cannot resurrect stale translations.

Keep mpv/libass rendering for rich ASS/SSA and bitmap subtitles until a tested adapter preserves their behavior. React owns generated/translated/dual text overlays. Route each selected source through exactly one renderer; native source subtitles can coexist with a translation overlay only through an explicit dual layout. Unified controls do not require one renderer for every format.

Use one media clock for in-window captions, with explicit epoch mapping across discontinuities, seeks, speed changes and output handoffs. Translation is segment-aligned; do not imply word-to-word timing across languages with different word order. Preserve original word timings for source highlighting and transcript search.

Cache identity must include:

| Cache | Required identity/version fields |
|---|---|
| Transcript | Profile, stable media identity + content revision, audio-track fingerprint, model artifact hashes, runtime/pipeline fingerprint, language/configuration, cache format |
| Translation | Source cue content/revision, source/target language, model + tokenizer hashes, inference configuration, cache format |
| Embedding | Profile/content revision, model + tokenizer hashes, dimensions, query/passage convention, preprocessing version, index format |
| Thumbnail | Content/video-track revision, generator/build fingerprint, dimensions, interval, cache format |

Do not continue relying solely on a URL hash for transcript identity: credential rotation changes URLs, and a stable URL can serve changed content. Unknown identity favors a cache miss over returning another item's captions. Namespace shared immutable model files separately from private per-profile derived content.

Invalidating transcripts invalidates dependent translations/search rows transactionally. Indexes can be rebuilt; favorites/history cannot be discarded. Model upgrades retain existing data until the new generation is valid and provide a clear rollback policy.

Use per-line language/direction, bidi isolation around mixed scripts and numbers, appropriate font fallback, measured wrapping and user-adjustable size/position/background. Avoid flipping transport controls merely because a caption is Arabic. Test Arabic, mixed Latin/Arabic, long unbroken text and missing glyphs. Caption changes must not pause video.

Live uses provisional/final cue revisions and rolling retention in the same timeline. Rewind replays captions from the matching capture epoch. Late translation must not flash after the corresponding utterance has already passed; expire it or make it available in the transcript.

## 11. macOS-native integration

React/Tauri retains browse, panels, controls, overlays, settings, transcript and diagnostics presentation. A small platform layer owns NSWindow behavior, render-surface lifecycle, fullscreen, menus, media keys, Now Playing, display changes, sleep/wake and accessibility preference observation. Prefer existing Rust/Objective-C bindings; add narrowly scoped Swift only where it simplifies a concrete public API integration.

All AppKit work runs on the main thread. The render context has an explicit owner and thread contract; detach and stop callbacks before destroying mpv or its drawable. Resize, Retina scale, display removal, fullscreen and closing must obey the same teardown rules.

Retain AVPlayer as a specialized AirPlay output adapter. Rust owns route selection, position transfer, failure rollback and audio exclusivity. Do not allow two audible engines. Verify actual receiver access to URLs: a receiver cannot generally fetch the Mac's loopback address. Test whether the chosen AirPlay mode relays data or requires a receiver-accessible route; never silently open the proxy to the LAN as a workaround.

Native PiP is a time-boxed feasibility task against public APIs and the pinned deployment target. A floating Nova mini-player is a valid fallback, but must be labeled as such. React caption overlays do not automatically travel with PiP or AirPlay. The capability matrix records which subtitle/caption modes each output supports.

Sleep suspends capture/recovery work according to policy; wake re-resolves Live and restores VOD position and user intent. Handle missing output devices and display transitions without assuming every wake means resume playback.

## 12. Player UX specification

| Situation | Required behavior |
|---|---|
| Uninterrupted playback | Video dominates; chrome fades after roughly 2.5 seconds idle, without layout shift |
| Pointer/keyboard interaction | Reveal chrome; keep it visible while focused, hovered, scrubbing or a panel is open |
| Paused | Stable transport controls, visible position, no recurring loading animation |
| Seeking | Immediate time/thumbnail/transcript preview; concise seek HUD; no flood of engine commands |
| Live without timeshift | Channel, LIVE, Now/Next, channel browser; no fake VOD seek bar |
| Live with timeshift | Actual seekable window, current position, known delay and Go Live |
| Movies | Minimal title, transport, rich scrubber, language panel; secondary controls in overflow/panels |
| Episodes | Show/episode identity, in-player browser, cancellable Up Next; trusted skip markers only |
| AI preparation | Subtle “Preparing captions…” or “Translating…”; playback remains available |
| Buffering/reconnect | Distinguish opening, temporary stall and retry; preserve context and show Retry/Back when exhausted |
| Fatal error | Plain explanation and meaningful next action; detailed diagnostics behind disclosure |
| Fullscreen | Native macOS fullscreen with consistent menu/keyboard/focus behavior |
| Theater | In-window layout mode; keep it distinct from fullscreen and mini-player |
| PiP/AirPlay | Show only supported routes and caption capabilities; preserve return position and pause intent |

Primary controls: play/pause, backward/forward seek where supported, timeline, volume and fullscreen. Secondary controls: unified Languages & Captions, episode/channel panel, playback settings, output route and diagnostics. Avoid a row of equally prominent feature icons.

The language panel groups Audio, Captions, Generate AI Captions, Translation target, Original/Translated/Dual display and Appearance. Generation/download states are described as user tasks, not queue jobs. Do not automatically download a large model just because the panel opens.

Keyboard defaults: Space/K play/pause; Left/Right seek; Shift+Left/Right larger seek; Up/Down volume except in the open channel selector; M mute; F fullscreen; C captions; Cmd+K command palette; Cmd+, settings. Escape dismisses the topmost panel before leaving fullscreen. Preserve existing advanced shortcuts when unambiguous; suppress playback shortcuts in text fields.

Single click on the video reveals controls; double click enters/exits fullscreen. This requested interaction supersedes the older repository skill's single-click pause behavior. Drag regions exclude controls. Trackpad gestures must not steal scrolling from panels or make system brightness a prerequisite.

Use restrained floating materials, visible focus, readable labels and touch-independent hover alternatives. Existing caption style/position preferences survive the redesign. The UI Lab includes every state above plus RTL, dense track lists, small windows and accessibility modes.

## 13. QoE and diagnostics

Capture a session-scoped event stream with monotonic timestamps, sequence numbers, session/generation IDs, playback policy and stack fingerprint. Instrument milestones at their actual ownership boundary.

| Measurement | Definition/source |
|---|---|
| Startup | User play intent → first presented frame when observable |
| Resolution | Resolver start/end, including credential/URL refresh |
| Connect/first packet/demux/decode | Record only if the pinned engine or transport exposes trustworthy signals |
| Presentation | Render callbacks/surface instrumentation; label first render submission as a proxy if actual presentation cannot be observed |
| Buffering | Count/duration after startup while playback is desired; exclude pause and explicit seek |
| Rebuffer ratio | Stall time divided by playing-plus-stall time, with denominator documented |
| Seek latency | Committed seek → observed target playback/frame; report error tolerance |
| Decode/render | Actual decoder, codec, resolution, FPS, bitrate when known, dropped frames, A/V sync |
| Live delay | Distance to measured playable target; distinguish it from broadcast-origin latency |
| Recovery | Attempts, URL refreshes, failure class, success/final failure |
| AI | Cold/warm model load, first cue delay, processing real-time factor, translation delay, cache hits |

Unknown metrics remain unknown. Estimated bitrate or first-frame proxies must be labeled. Never claim broadcast latency from buffer size alone.

Keep a bounded in-memory detailed ring plus local per-session summaries. Initial policy: seven days and 50 MB for diagnostics, configurable and deletable. No raw stream URLs, headers, cookies, credentials, transcript text or viewing titles in exported diagnostics by default. Aggregate failures using opaque identifiers. Telemetry upload is absent unless explicitly introduced with opt-in.

“Copy Diagnostics” combines the embedded build/stack report with observed runtime versions, macOS/hardware, codec/output data and recent redacted failures. Allow preview before copying/exporting. Retain stack and schema fingerprints without personal data.

Phase 0 establishes a reference Apple Silicon machine and fixture baselines. Initial goals include sub-100 ms control feedback and the existing live tune target below 800 ms, with 1.2 seconds as a controlled-fixture ceiling. These are not promises for arbitrary providers. Add p50/p95 startup, seek, AI and resource measurements, then approve numerical gates against real baseline results. A suggested relative regression trigger is over 15% on repeated same-hardware runs; investigate variance rather than accepting or rejecting a single noisy run.

## 14. Environment and stack management

Adopt **one authority per input and one generated consolidated stack lock**, rather than a second hand-maintained list of versions.

| File | Authority |
|---|---|
| `Cargo.toml` workspace package version | Application version; generate/validate desktop package and Tauri versions from it |
| `rust-toolchain.toml` | Exact Rust toolchain; Cargo/rustc versions observed and recorded |
| `mise.toml` | Exact Node and just versions; install Rust by reading its authority, not another manual Rust pin |
| Root `package.json#packageManager` | Exact pnpm version; bootstrap uses this instead of a competing mise pin |
| Cargo manifests + `Cargo.lock` | Dependency constraints/features and exact Rust resolution |
| Package manifests + `pnpm-lock.yaml` | JS constraints and exact resolved packages/integrities |
| `manifests/native.lock.toml` | Native source revisions, build recipes/options, architectures, artifact URLs/hashes, transitive native graph and licenses |
| `manifests/models.lock.toml` | Exact model/tokenizer revisions, files, sizes, hashes, quantization and conversion provenance |
| `manifests/platform.toml` | Minimum macOS, approved build Xcode/SDK, deployment targets, runner-image policy and architecture matrix |
| Immutable SQL migrations | App/profile migration history and checksums |
| `manifests/data-formats.toml` | Cache/index/IPC versions and compatibility ranges, consumed by generated Rust/TS constants |
| `manifests/stack-policy.toml` | Critical components, required features, tiers, permitted environments and validation rules |
| `nova-stack.lock.json` | Generated normalized composition of the above; reviewed, committed and never hand-edited |
| `build-environment.json` | Observed build/runtime report; generated per build, embedded and archived |

Version declarations in ecosystem files remain necessary constraints. Resolve duplicates through generation or CI validation. Cargo.lock pins versions but not every build feature, system library or SDK; the consolidated report must include selected features, target and native provenance too.

The generated lock's conceptual structure is:

```text
schema_version
input_digests                         # manifests, lockfiles, recipes
application.version
toolchains.{rust,cargo,node,pnpm,just}
frontend.{react,typescript,vite,tauri_js,critical_packages}
rust.{tauri,sqlx,critical_crates,features}
native[]                             # versions, revisions, flags, hashes, arch
ai[]                                 # runtime + model + tokenizer fingerprints
platform.{deployment_target,xcode,SDK,architectures}
data.{app_migrations,profile_migrations,indexes,caches,IPC}
stack_fingerprint                    # canonical serialization hash
```

Do not place the current Git commit inside a committed generated file that changes on every commit; that creates self-reference churn. The separate build report adds Git commit/tag, dirty state, release channel, workflow SHA, runner image version, compiler/SDK observations and artifact fingerprints. Signing and notarization results are appended to an external release record after packaging.

Commands, initially implemented through `just` and a small `xtask` tool:

- `just stack`: report expected versions; `--json` emits the generated representation.
- `just doctor`: compare expected, installed and loaded components; show actionable missing-tool instructions without upgrading anything.
- `just verify-environment`: strict check for the selected development/CI/release profile.
- `just stack-check`: regenerate in a temporary directory and fail on differences; never silently bless drift.
- `just stack-update`: intentional maintainer operation after source manifests change; its diff belongs in the upgrade PR.

Examples of failures: mismatched pnpm pin; stale frozen lockfile; native mpv artifact hash mismatch; model files differing from approved revision; unsupported Xcode build; app version disagreement; generated stack lock out of date. An absent optional model is valid until a test/feature requires it; a present corrupted model is not valid.

Pin exact patch toolchains after inventorying a working machine and clean CI build; this plan does not invent installed versions. Report actual WKWebView/WebKit and VideoToolbox behavior through macOS versions because these OS frameworks cannot be pinned like bundled libraries. Test the supported OS matrix separately from the single approved build environment. Record runner image revisions and Xcode build IDs: a `macos-*` label alone is not an immutable machine image.

All native/model artifacts require immutable sources, SHA-256, architecture, license evidence and provenance. Verify before extraction/load, install atomically, and record selected inference options. Checksums must originate from a trusted reviewed manifest; fetching an arbitrary checksum beside an arbitrary file is not provenance.

The stack report is included in every release and diagnostic export. Model installation state and actual active model hashes are runtime observations alongside the build's approved model set. Models update deliberately and do not silently change behind a fixed app version.

## 15. Repository/build architecture

Keep existing crates. Add modules first and extract only when their dependencies and ownership are clear:

```text
apps/desktop/src/features/player/       presentation components + adapters
apps/desktop/src-tauri/src/playback/    coordinator during initial migration
apps/desktop/src-tauri/src/platform/    macOS bridge/output integration
crates/playback/                        eventual pure session/policy domain
crates/mpv-player/                      engine/FFI implementation
crates/captions/                        eventual timeline + adapters
crates/{m3u,xtream,epg,storage,...}/     existing subsystems retained
tools/xtask/                           stack/environment/build orchestration
manifests/                             native/model/platform/data policy
tests/provider-fixture/                deterministic HTTP provider
tests/media/                           manifests + small redistributable assets
tests/e2e/                             real desktop flows
tests/visual/                          Player UI Lab baselines
docs/adr/                              accepted decisions
```

Avoid a giant move-only refactor. Move the existing `docs/decisions` convention to `docs/adr` only when adding the first records; redirect any existing links.

Use `just bootstrap`, `just check-all`, `just dev`, `just test-e2e`, `just package`; keep current commands as aliases where practical. Optional root wrappers delegate to these, rather than implementing a second build system. Bootstrap is idempotent, installs declared dependencies, validates hashes and uses frozen/locked resolution. It lists Xcode/license prerequisites explicitly and does not hide privileged system changes.

Native build output lives in an ignored content-addressed cache or verified artifact store, not Git. Build recipes pin compilers/tools and third-party sources; development, CI and release consume the same native SDK artifact. `build.rs` resolves this SDK explicitly and fails rather than silently linking Homebrew. Bundle the full non-system dylib closure with controlled `@rpath`/`@loader_path` references; include FFmpeg/ffprobe and whisper-cli where used.

AI weights remain optional downloads outside the app bundle, with compatibility records and quotas. Build-time model conversion can use a separately locked Python environment if necessary; Python is not a user runtime dependency. Disable libraries' implicit model/native-runtime downloads in strict builds.

Never modify a published migration. Back up using a SQLite-consistent mechanism before destructive transitions; copying a live WAL database file alone is insufficient. Check supported reader/writer schema ranges before opening a newer database. Unsupported downgrade fails safely with recovery guidance, rather than using `ignore_missing` as permission to write unknown schemas. Test migration and restore paths.

`README` provides onboarding; `ARCHITECTURE` describes accepted boundaries; generated `STACK.md` reports versions; `ROADMAP` owns feature status; `CHANGELOG` owns releases. `docs/testing.md`, `docs/releasing.md` and `docs/environment.md` own procedures. Agent instructions link to these and contain conventions, not a second roadmap.

## 16. Version-control strategy

Git is canonical and GitHub hosts the authoritative shared repository. Protect `main`, require PR checks, use short-lived branches and squash/rebase for readable history. Keep feature flags short-lived and record removal criteria. Optional local Jujutsu must produce ordinary compatible Git commits; CI and release tooling never depend on it.

Target meaning of `main`: required regressions passed and an installable development/alpha package can be produced from declared inputs. Public notarization/signing is a separate protected release operation; PRs never need signing secrets.

Use annotated release tags and SemVer prereleases for alpha/beta/stable. Channels are distribution metadata, not long-lived development branches. Avoid cherry-pick-heavy GitFlow. If maintaining an older stable line becomes necessary, define a narrow support branch policy then.

ADRs cover desktop shell, libmpv/rendering, state ownership, profile storage, local AI, native artifacts, caption timeline, stack manifest, upgrade policy and distribution. Each records alternatives, decision, evidence, consequences and conditions for reconsideration.

## 17. Dependency upgrade strategy

Classify by impact, not only package category: a decoder patch can be riskier than a formatter major. Tier 1 includes media, Tauri/native bridge, Rust, SQLite and AI runtimes/models. Tier 2 includes React/TypeScript/Vite and major UI/search libraries. Tier 3 includes development-only utilities; their generated-output impact still matters.

| Upgrade | Required additional regression evidence |
|---|---|
| Rust/toolchain | Workspace tests/lints, native FFI build, packaged launch, ABI/link review and performance comparison |
| Tauri/plugins | Real IPC/events, profiles/filesystem, windows/fullscreen, native render bridge, signing/package/updater smoke |
| React/UI packages | Component tests, keyboard/focus, UI Lab visuals, real WKWebView smoke, player render/update profiling |
| Node/pnpm/TypeScript/Vite | Frozen dependency install, typecheck/build, generated assets, CSP behavior and packaged frontend smoke |
| mpv/FFmpeg/libplacebo/libass | HLS/TS/direct files; H.264/HEVC/4K; hardware failure fallback; tracks/subtitles; seek/resume; stalls; render lifecycle; PiP/AirPlay where supported |
| SQLite/sqlx | Historical DB upgrades, FTS correctness, transactions, refresh preservation, corruption/error handling, compatibility refusal |
| Whisper/runtime/model | Language quality/WER fixtures, silence/hallucination cases, word timing, first cue delay, seeking, track changes, stitching, cached replay |
| CT2/translation/model/tokenizer | EN→FR, EN→AR, FR→EN, quality review/chrF-type metrics, latency/memory, source token handling, RTL, cache invalidation |
| ONNX/fastembed/E5 | Multilingual relevance, query/passage preprocessing, dedup false merges, profile isolation, equal-count index updates, memory |

Upgrade process: inventory current baseline → open isolated upgrade PR → update authoritative pins/artifacts → regenerate stack lock → summarize changelog/breaking changes → run relevant matrix → compare metrics → review → promote through alpha. Major architecture replacements require an ADR and rollback/migration plan.

Use Renovate/Dependabot to propose grouped updates. No automatic merging of critical runtime changes. Keep the native media dependency closure coherent, but avoid combining it with unrelated AI/UI upgrades. PRs include old/new versions, affected subsystems, exact test environment and evidence. Emergency security updates accelerate this process without bypassing the essential compatibility tests.

## 18. Testing architecture

| Layer | Purpose |
|---|---|
| Unit | Policies, state transitions, timestamp mapping, normalization, caption stitching, cache keys, retry classification |
| Component | Accessible controls, panels, visibility, keyboard behavior, empty/error states and presentation-only mocked adapters |
| Integration | Real SQLite, migrations, provider HTTP, job cancellation, engine command sequencing, model artifact verification |
| Desktop E2E | Built Tauri app with real Rust/SQLite/mpv and fixture network; no mocked playback IPC |
| Visual | Frozen UI Lab scenarios and whole-window native screenshots |
| Hardware | Render/decode/HDR, native fullscreen/PiP, AirPlay, media keys, display and sleep/wake |

Use pinned `@wdio/tauri-service` with its embedded WebDriver plugin for a macOS test build. Standard direct `tauri-driver` remains a different path without equivalent macOS support. Enable the driver only through a test feature/configuration, bind narrowly, and verify it is absent from release builds. Browser-mode tests remain component tests, not desktop E2E. [Tauri testing setup](https://v2.tauri.app/develop/tests/webdriver/)

Native video may not appear in a DOM screenshot. A real smoke must verify decoded/rendered progress and independently capture the composed window or a known fixture frame. Add XCTest/accessibility-driven native checks where the embedded driver cannot operate system UI. Test instrumentation helps observe production paths but must not bypass resolution, storage or playback.

Create a local deterministic provider with M3U, Xtream category/list/episode endpoints, XMLTV, short movies, ordered episodes, live HLS/TS and range requests. Include multi-audio, subtitles, predictable speech, timestamps, redirects, expiring URLs, headers, byte ranges, delayed responses, disconnects, invalid manifests, discontinuities and HTTP failures. Keep fixture content redistributable, hashes fixed, ports dynamic and state isolated. A generated large catalog covers the 40k+ item scale without real credentials.

Separate deterministic inference fakes for orchestration tests from real-model suites. Real speech fixtures have reference transcripts, word timing tolerances and reviewed translations; avoid treating exact generated strings as the sole quality criterion. Track quality and latency per pinned runtime/model/hardware combination.

Required PR CI: fmt, clippy, Rust tests, typecheck, lint, Vitest, migration/contract tests, frozen locks, stack verification, security/license checks, package-linkage audit and at least one real macOS fixture smoke. The smoke covers profile/import/browse/play/seek/relaunch/resume and stays short; it does not download multi-gigabyte models every PR. Critical AI changes require the cached real-model suite before merge.

Nightly: the full golden flow, real models, seek/track-switch stress, refresh preservation, profile isolation, download retry/corruption, offline behavior, network faults, long playback, resource bounds and UI visuals. Use trusted GUI-capable Mac runners; physical-device jobs cover sleep/wake/AirPlay/HDR. Never run untrusted fork code on a persistent Mac containing secrets or user data.

Golden flow:

```text
isolated fresh profile -> import fixture provider -> browse/find movie
 -> play/seek -> generate real captions -> translate -> quit/relaunch
 -> resume -> search dialogue -> select result -> verify target position
```

The Player UI Lab supplies movie paused/playing/error/seeking, live reconnecting/behind-edge, episode up-next, Arabic/dual/long captions, dense tracks, window sizes and accessibility modes. Freeze time, fonts, data and animation for screenshot baselines. DOM visuals and native-composition visuals have separate baselines; approvals require reviewing the change, not bulk accepting images.

Release tests install the actual signed/notarized artifact on a clean Mac without Homebrew/runtime tools. Exercise it externally without shipping a test driver. Qualify supported macOS versions, update from the previous supported release and verify data preservation. A compile-only or mocked CI pass is not a release certificate.

## 19. Release architecture

Distribute directly as a notarized Apple Silicon `.app`/DMG first. Keep macOS 14 as the intended minimum until the pinned dependency graph and test matrix prove it; raise it only through an explicit support decision. Intel/universal and Windows are separate qualified targets.

Release order:

1. Build from a clean reviewed commit/tag with locked dependencies and approved native SDK.
2. Generate build report, stack lock copy, notices, SBOM and provenance inputs.
3. Assemble bundle; audit dylib closure, executable paths, deployment targets and missing resources.
4. Sign nested code and application correctly with Developer ID/hardened runtime; apply only necessary entitlements.
5. Test the signed candidate, then package and notarize; staple and verify signature/Gatekeeper behavior.
6. Install and test the final distributed artifact on a clean system; verify no test hooks or development dependencies.
7. Compute final checksums and updater signatures; archive artifacts and metadata.
8. Publish only through an approved release action and promote channels without rebuilding equivalent artifacts.

Signing changes artifact bytes. Reproducibility initially means traceable identical inputs and explainable unsigned build outputs; do not promise bit-identical notarized packages. Archive pre-sign and final checksums separately. [Apple distribution signing](https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac/), [Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)

Use Tauri's updater with signed update artifacts and HTTPS endpoints separated by channel/architecture. Do not assume its standard JSON feed is itself signed: preserve artifact signature verification, protect feed integrity and add separately authenticated release metadata if the threat model requires it. Protect updater keys separately from Apple signing keys. [Tauri updater](https://v2.tauri.app/plugin/updater/)

Archive commit/tag, both lockfiles, manifests, native recipes/sources, model revision hashes, toolchain/SDK/runner observations, workflow/action revisions, schemas, test reports, SBOM, signing identity metadata without secrets, notarization results and final checksums. Ensure native artifacts remain retrievable; a Git tag plus dead download URL is not a reproducible release.

Rollback is constrained by database compatibility. Prefer forward fixes; if reverting the binary, enforce schema compatibility or restore an explicit consistent backup with user-visible consequences. Never silently downgrade data.

## 20. Security and supply-chain hardening

- Define a production CSP for packaged assets and permitted artwork/network paths; remove unrestricted navigation and do not render provider text as HTML. Keep dev-server allowances separate.
- Scope Tauri capabilities to actual windows/actions. Validate profile, media, session and resource ownership in backend commands; frontend types are not authorization checks.
- Keep provider/TMDB credentials in Keychain-backed storage; database rows hold references. Redact URL paths as well as query strings because Xtream credentials can occur in both.
- Enforce supported schemes, input sizes, redirect/header rules, timeouts and parser limits. Do not let playlist entries invoke arbitrary local files or shell commands. LAN IPTV must remain possible through explicit source policy, not a blanket private-network prohibition.
- Keep local proxy routes session-token scoped and loopback-only by default; prohibit arbitrary proxy URLs, unsafe CORS and path traversal. External receiver access is a separately designed capability.
- Invoke subprocesses with typed argument arrays; bound lifetime/output and avoid credentials in command logs. Where tools require credentials in arguments, minimize exposure and prefer protected broker/transport handles where supported.
- Verify archives before extraction, constrain entries/size/symlinks, install into staging and atomically activate. A model is installed only after its manifest files and hashes validate.
- Audit Rust/JS/native/model licenses and advisories. Use cargo-deny and equivalent JS checks, pin CI actions by commit, record exceptions with owners/expiry, and generate an SBOM covering more than Cargo/npm.
- Resolve the exact mpv/FFmpeg build's licensing obligations, including enabled dependencies and source/relinking requirements. Choosing an LGPL-compatible configuration is a build decision, not something inferred from a package name. [mpv copyright](https://github.com/mpv-player/mpv/blob/master/Copyright), [FFmpeg licensing](https://ffmpeg.org/doxygen/trunk/md_LICENSE.html)
- Test release bundles for absent dev driver/MCP endpoints and unwanted shell/network access. Keep signing secrets out of PR jobs.

Mac App Store distribution is not the initial target. Sandbox, downloadable artifacts, media dependencies and licensing require their own feasibility review; do not distort the working architecture around an uncommitted store requirement.

## 21. Phased migration plan

These phases are acceptance gates, not promises of a particular number of days. Do not upgrade the media runtime at the same time as replacing the coordinator or renderer.

### Phase 0 — Environment, architecture and dependency baseline

**Objective:** make today's app reproducible and observable before changing its player architecture.

**Changes:** inventory exact resolved packages, installed/loaded native libraries and model artifacts; establish authority files, stack lock, exact toolchain pins, native/model provenance, bootstrap, doctor and strict drift checks. Record ADRs and upgrade policy. Build the fixture provider and macOS driver spike. Create a clean installable development bundle and document current bugs/performance. Fix high-risk profile-cache isolation and schema-compatibility issues with focused tests as confirmed.

**Tests:** clean bootstrap on a second environment, lock/manifest disagreement failures, artifact corruption rejection, native linkage scan, existing tests, real desktop smoke.

**Exit:** one declared command chain creates and exercises an installable app without undocumented Homebrew libraries; all critical dependencies are identified and pinned, CI detects intentional drift injections, a baseline report is retained. NLLB/native licensing decisions have an owner and a release gate.

**Risks:** native build complexity, undocumented machine dependencies, misleading old documentation. Do not claim exact reproducibility until a second clean environment succeeds.

### Phase 1 — PlaybackSession foundation

**Objective:** centralize authority without changing normal user behavior.

**Changes:** coordinator actor, typed kinds/state/recovery, resolver, output capabilities, sequence/generation contracts, stable identity, cancellation and Rust-owned resume/output handoff. Wrap existing mpv commands rather than replacing the renderer. Add clock/QoE anchors.

**Tests:** transition tables, reordered/stale callbacks, rapid open/close/switch, profile switch during AI/resolution, pause during recovery, native teardown, existing golden smoke.

**Exit:** one owner controls playback; no obsolete event changes the current session; switches release resources; resume survives relaunch; React no longer orchestrates engine recovery.

**Risks:** lifetime races, FFI deadlocks and changed behavior. Keep a short-lived rollback flag and remove it after parity evidence.

### Phase 2 — Live reliability

**Objective:** dependable tuning and recovery with truthful live status.

**Changes:** LivePolicy, bounded retries, URL refresh, buffer presets, measured live target, diagnostics, provider compatibility cases, channel selector and EPG binding. No new DVR dependency.

**Tests:** HLS/TS, redirects/auth, discontinuities, outages, repeated tuning, hardware fallback and long playback; reference-machine p50/p95 comparisons.

**Exit:** recovery preserves intended channel/state, retries terminate, resources remain bounded and controlled-fixture tune goals are met or explicitly revised from evidence. Unknown edge/latency is displayed honestly.

**Risks:** provider variability and connection caps; keep fixture and opt-in real-provider results distinct.

### Phase 3 — Player frontend redesign

**Objective:** a coherent, accessible macOS player over the stable coordinator.

**Changes:** split PlayerModal, unified language panel, primary/secondary controls, Live/VOD timelines, focus/keyboard behavior, UI Lab, visual tokens and diagnostics view.

**Tests:** component/visual coverage for every canonical state, real WKWebView integration, VoiceOver and keyboard checks, reduced motion/transparency, Arabic layout, native fullscreen/resize.

**Exit:** no lost existing controls, no layout shifts, no hidden focused controls, no 60 Hz tree updates, all mode-specific affordances match capabilities.

**Risks:** visual polish masking state errors and native/WebView composition issues. Validate the composed window as well as DOM screenshots.

### Phase 4 — VOD/Series polish

**Objective:** reliable seek/resume and episode continuity.

**Changes:** coalesced seeks, thumbnails/chapters, transcript previews, next-episode countdown, bounded metadata preload and preference hierarchy; native output feasibility gates.

**Tests:** exact/approximate seek capabilities, pause/resume, duplicate end events, cancelled autoplay, expired episode URLs, audio changes, output handoff.

**Exit:** natural end advances once; Cancel reliably cancels; seeking/resume meets fixture tolerances; preloading respects provider limits. Unsupported PiP/HDR/markers remain visibly capability-gated.

**Risks:** expensive thumbnail generation, wrong track mapping and overpromising native output features.

### Phase 5 — Caption unification and AI qualification

**Objective:** consistent caption timing, selection and caches across sources.

**Changes:** timeline adapters, source revisions, format-aware render routing, stable cache keys, shared AI resource budget, model lifecycle validation, transcript/search integration and licensed translation decision. Keep existing Whisper stitching until alternatives prove better.

**Tests:** actual EN/FR/AR speech/translation, silence and long dialogue, cache replay, seeks/tracks, Arabic/dual visuals, embedded ASS/bitmap behavior, model cancellation/retry, migration/invalidation.

**Exit:** full real-model golden flow passes on reference hardware; timing/quality/resource reports meet approved budgets; captions never interrupt playback; production model artifacts have approved provenance and intended-use rights.

**Risks:** translation quality/licensing, CPU contention, stale source revisions and format loss. Stage backend replacements behind the existing trait if needed.

### Phase 6 — Optional timeshift/DVR and live AI

**Objective:** add shared capture only after direct playback is stable.

**Changes:** rolling segment store, time/epoch map, disk quotas, recording lifecycle/scheduler, PCM tap and bounded incremental ASR/translation.

**Tests:** disk full, crash recovery, eviction/read races, discontinuities, recording conflicts, sleep/network loss, provider limits, live rewind/caption alignment.

**Exit:** bounded disk/network usage, correct rewind/Go Live, honest missed-recording reporting and a measured live AI latency budget. DVR/live AI may ship independently behind explicit capabilities.

**Risks:** substantially larger media pipeline, provider load and false expectations around background recording. This phase does not block an alpha, beta or initial stable release without DVR.

### Phase 7 — Distribution hardening

**Objective:** promote a proven build through alpha/beta/stable.

**Changes:** production signing/notarization, updater, complete license notices/SBOM, final provenance/archive, clean-install and upgrade qualification, removal of test hooks.

**Tests:** final artifact on minimum/current supported macOS, Gatekeeper/signature, absent Homebrew dependencies, real model installation, update interruption, schema compatibility and channel isolation.

**Exit:** approved artifact and updater signature verify; exact stack can be reconstructed; installation/update preserves data; no unresolved release-blocking reliability or licensing findings.

**Risks:** signing identity, nested dylibs, downloadable-model compatibility and unsafe downgrades.

**Sequencing:** Phase 0 already includes packaging feasibility; Phase 7 infrastructure begins there and hardens continuously. Ship validated alpha after core Phases 1–5 and the relevant release gates. Phase 6 is optional and must not postpone release readiness.

## 22. Prioritized backlog

| Priority | Work and acceptance evidence |
|---|---|
| Must do now | Exact stack/native/model inventory; doctor/drift checks; clean bootstrap/package spike; licensing decision; real macOS smoke; targeted profile-cache and migration-compatibility tests; truthful PiP/renderer/AI documentation |
| Before validated alpha | Coordinator/resolver/cancellation; bounded Live recovery; golden real-model flow; model verification; basic diagnostics; unified player controls; track/cache isolation; signed clean-machine package; CSP/credential boundaries |
| Before beta | Expanded provider/codec/device matrix; sustained playback and AI benchmarks; migration/update qualification; visual/accessibility regressions; durable enrichment jobs; validated thumbnails/episode UX; supported output-route tests |
| Before stable | Repeated release qualification; tested updater/recovery; complete license/SBOM/provenance; bounded resources and data retention; supported OS policy; no unexplained critical failures or production test hooks |
| Later / optional | EPG grid if not pulled forward, timeshift/DVR, live AI, multiview, skip-marker detection, richer native PiP/HDR controls, additional outputs/platforms, cloud translation, local LLM cleanup |

Do not make feature breadth a substitute for the alpha acceptance bar. Existing caption/translation/search features must either pass their validation gate or be clearly marked experimental/disabled in the relevant release channel.

## 23. Risk register

| Risk | Impact / mitigation / gate |
|---|---|
| mpv embedding lifetime | Critical crashes; explicit render-thread ownership, teardown order and repeated native lifecycle tests before coordinator rollout |
| OpenGL/macOS composition | Black/glassy frames, future compatibility; retain working path, test resize/scroll/display/fullscreen, isolate renderer adapter, require ADR for replacement |
| mpv/FFmpeg/libplacebo drift | Codec/startup regressions; coherent verified native SDK and upgrade-specific compatibility suite |
| VideoToolbox behavior | Codec/device-dependent failures; measure actual decoder, recover to software, test supported hardware |
| HDR assumptions | Incorrect colors or clipped highlights; certify end-to-end output separately from codec decode |
| Native PiP/AirPlay | Unsupported presentation/captions or failed URL reachability; capability matrix, real device tests, specialized adapter and honest fallback |
| Native/model licensing | Distribution blocker; exact build/model review, source/notices obligations, approved translation backend before intended commercial distribution |
| Model revision/tokenizer drift | Silent changed text or incompatible caches; immutable revisions/hashes, quality evaluation and cache fingerprints |
| Provider variability | Auth/redirect/header/TS issues; central resolver, fixture fault cases, bounded provider-specific policy |
| Provider connection limits | AI/preload/DVR disrupt playback; shared capture where appropriate and connection budgeting |
| Live latency claims | Misleading measurements; separate broadcast origin, playable edge and buffer estimates; unknown when unobservable |
| Cache/profile identity | Wrong captions/search results across profiles; scoped keys, generations, content/track fingerprints and isolation tests |
| Schema downgrade | Corruption/data loss; compatibility ranges, backups, refusal of unsupported writes and restore tests |
| Timeshift disk pressure | Exhausted storage/data loss; quotas, pinned reader references, crash-safe metadata and disk-full tests |
| React/Rust desynchronization | Controls command wrong session; one authority, sequence/generation guards and snapshot recovery |
| In-process inference | Memory pressure or native-runtime crash affects app; bounded concurrency, idle unloading, stress tests; process isolation if evidence requires |
| Tauri/native accessibility | Correct DOM but broken native experience; real VoiceOver/keyboard/composed-window testing |
| Environment drift | Undiagnosed regressions; generated lock versus observed report, strict CI checks and no Homebrew fallback |
| CI/release divergence | Tests pass but installer fails; same native artifacts, release-like smoke, final signed artifact tests |
| Test harness exposure | Debug endpoint in production; compile-time exclusion and release artifact inspection |
| Secret/diagnostic leakage | Provider/account exposure; Keychain, sensitive types, URL redaction, opt-in export and tests |
| App Store constraints | Late distribution redesign; direct distribution first, separate store feasibility ADR |
| Scope expansion | Reliability work never ships; phase gates and explicit exclusion of DVR/live AI from initial release bar |

Ownership follows the boundary: playback/native owner for rendering/output risks; data owner for schemas/identity; AI owner for models/quality; build/release owner for artifacts and licensing records; frontend owner for interaction/accessibility. One person may hold several roles, but each gate needs a named owner when scheduled.

## 24. Final recommended architecture

Nova should become a reproducibly built Tauri/React application with a Rust-owned playback session, a pinned libmpv media runtime, per-profile SQLite storage, a shared caption timeline and replaceable local AI adapters. A narrow native macOS layer handles the behaviors a web surface cannot reliably provide. The stream resolver and optional capture service absorb provider variability without rewriting the player.

This preserves IPTV compatibility and the features already built while making failure recovery, resource use, state ownership and output capabilities explicit. It keeps the player responsive by separating presentation from engine control and giving playback priority over inference. It makes releases explainable through a generated stack lock, observed environment reports, verified artifacts and archived provenance.

The implementation order is deliberate: establish a reproducible baseline and real test evidence; consolidate playback authority; harden Live; refine the player experience; qualify VOD/Series and captions; release a validated alpha. Add timeshift, DVR and other premium features through those established boundaries when their own acceptance gates can be funded and met.

The target is a known, pinned, validated stack that can evolve intentionally—without sacrificing the working Nova application to a rewrite.
