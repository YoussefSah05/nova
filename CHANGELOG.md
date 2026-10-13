# Changelog

## 0.2.0-alpha.0 — in development

- Fix five Settings actions that silently did nothing on macOS: removing a playlist, deleting a
  profile, removing the Whisper model, clearing a cache and removing the TMDB token. They asked
  through `window.confirm`, which the macOS WebView never shows; they now confirm in an in-app
  dialog with Cancel as the default.

- Word-accurate captions: a new caption timing stage runs around Whisper in the ASR worker.
  Silero voice detection skips windows without speech and drops invented lines; an MMS-300M
  forced aligner places every word on the audio (English, French and Arabic through one model).
  Mean word-start error falls from 398 ms to 50 ms on 779 reference words. Install it from
  Settings → Models → Caption timing (~320 MB, pinned and checksum-verified).
- Captions without the cold-start wait: films and episodes now decode on the resident speech
  worker instead of starting `whisper-cli` for every window. The worker starts on the GPU while
  the Neural Engine model compiles, warms up when the playback menu opens, and the first window
  is short. Opening a title's playback menu, or a title when captions are on by default,
  transcribes its opening before Play, and the player takes that work over. First caption on a
  cold start: 32.1 s → 2.1 s.
- Fix word timing never using DTW: whisper.cpp 1.8's default flash attention disabled it.
- Keep captions on screen long enough to read (at least 1 s, and 0.4 s after the last word),
  and show the newest line when two overlap.
- Add ADR 0014, a caption timing evaluation with reproducible harnesses, and a recorded demo
  of the intelligence features on *Tears of Steel* (CC BY 3.0, Blender Foundation).

- Show translated and Source + translation captions at the bottom of the picture, like source
  captions, instead of in a panel over the middle of the video; both lines follow the caption
  appearance settings (size, font, colour, shadow / outline / box).
- New Nova logo, "the star that brightens": app icon, boot screen, top bar, launch and welcome
  screens, empty library, Settings → About and README, with a brand guide.
- Run the interface at 120 Hz on ProMotion displays (Settings → Appearance → High refresh
  rate), lifting WebKit's default 60 fps pacing.
- Add a frame-rate overlay (⌘⌥F) that records per-screen scroll smoothness with a copyable
  report.
- Smooth scrolling at 120 Hz: shared grid context menus, off-main-thread and persisted
  channel-logo colours, cheaper show-card text, accurate virtual row sizes, eager image loading
  in virtualized grids.
- Stop showing the removed NLLB model as an installed translation direction.
- Add a user guide, FAQ, roadmap, performance engineering notes, ADR 0013, a code of conduct,
  and issue and pull-request templates.
- Make Live fast on large playlists: a guide page drops from ~73 s to ~2 ms and channel
  switching from ~5.5 s to under 1 ms on an 8.7k-channel playlist (profile migration 0046).
- Group live channels into families with source-scoped variants (edition, language, region,
  quality, codec), a bundle detail page, a variant picker with per-variant health and a
  remembered choice (profile migration 0044).
- Install the four Opus-MT translation directions from Settings with commit-pinned,
  checksum-verified downloads; all four directions and the French ↔ Arabic pivot measured on
  real models.
- Reuse catalog embeddings across playlist refreshes by model and exact text (profile
  migration 0045): an unchanged ~37k-title refresh goes from ~3 minutes of inference to under
  a second.
- Fix release bundles failing to link whisper.cpp's Metal backend.
- Add verified audio-track selection and caption start choices; translated captions
  (Translation / Source + translation) in English, French and Arabic; persistent model
  controls with caption readiness status; a private, checksum-pinned Whisper CLI runtime.
- Rewrite the README and add an architecture overview, an intelligence guide, a getting-started
  guide, a contributing guide and a security policy.
- Add encrypted catalog derivation storage and cache-first reuse with exact context,
  artifact-version and expiry checks; serving-app integration remains pending.
- Add an unconnected Rust catalog reasoner with bounded local inference, forbidden-link
  and metadata-conflict checks, and an expiring provisional-evidence bridge.
- Compare pinned Qwen and Ministral models on development and frozen synthetic
  transfer cases; choose Qwen 4B/v2 as a development reference, not a serving-app model.
- Add a development catalog-model comparison plan, synthetic multilingual smoke
  corpus and bounded local inference evaluation harness with CI regression tests.
- Establish required catalog intelligence in the plan and add an unconnected,
  strict proposal/evidence evaluator with conflict, expiry and authority checks.
- Add a development stack manifest and read-only stack/environment verification commands.
- Pin selected Rust/Node/pnpm/just versions; use locked package resolution and explicit bootstrap.
- Add environment drift regression tests and CI manifest validation.
- Catalog and player runtime behavior remain on the existing implementation.
- Add unconsumed catalog source contracts with typed IDs, bounded preserved evidence,
  explicit unknown numbering and diagnostic redaction; no profile migration yet.
- Add bounded snapshot capture, immutable completion receipts and safe per-endpoint
  partial-refresh selection, plus versioned SHA-256 metadata/locator fingerprints.
- Add a synthetic loopback IPTV provider and real-client integration tests for M3U,
  Xtream, lazy episodes and XMLTV.
- Correct M3U episode suffix offsets for Unicode titles; regression tests pass,
  with real-provider and running-app validation still outstanding.
- Add an unconnected encrypted snapshot file-store prototype with authenticated
  replay, private files, no-overwrite commits and process-interruption tests.
- Add profile snapshot-key provision/load contracts and a macOS Keychain adapter;
  native provisioning still requires signing-entitlement validation.
- Add v2 streaming encrypted snapshots with authenticated final receipts, bounded
  queues and conservative byte/file quotas; automatic eviction remains pending.
- Add an unconnected Xtream object adapter and encrypted record archives that
  preserve raw evidence, duplicate upstream occurrences and bounded source fields.

## 0.1.0 — pre-migration historical baseline

- Preserve the existing Nova application and architecture proposals before production hardening.
- Record the existing Cargo dependency resolution and available validation evidence.
- Existing capabilities include provider import, browsing, native playback, local captions,
  translation and search; this checkpoint is not a publicly certified release.
- Known validation gaps: pre-existing Rust formatting differences, no actual-app E2E,
  no clean packaged-app/device certification, and unverified remote CI/protection.
- See `docs/implementation/baseline.md` for environment caveats and evidence.
