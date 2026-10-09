# M18 — committed caption translation (2026-10-09)

## Implementation contract

Reuse `CaptionSessionManager`, persisted transcript segments, `TranslationRepository`,
the bundled manifest's direct/pivot routing, and runtime-owned translator leases.
The engine exists; the missing behavior is translating actual committed dialogue
and displaying it in the player.

- Rust emits committed segment IDs and the transcript ID with source cues. A new
  command accepts at most eight IDs, verifies they belong to the active caption
  session/profile/transcript, and reads text from storage rather than trusting IPC text.
- One translation batch per caption session; model work uses Realtime admission.
  Stop/media/track changes cancel through the existing session token. Target changes
  discard obsolete UI results; a completed valid result may still populate the cache.
- Cache lookup precedes model acquisition. The existing `backend` key stores serialized
  provenance: pipeline/decoding version, source language/text SHA-256, target, and
  every route model's ID/revision. Legacy cache entries remain untouched and cannot
  masquerade as newly versioned results. No migration or model download is required.
- Restore Translation/Dual modes for English, French and Arabic in the caption picker,
  with RTL rendering and source fallback during missing/offline models or failures.
- Verify bounded/foreign-ID rejection, cache/version invalidation, direct/pivot output
  validation, cancellation and stale-result rejection, plus player rendering.

Non-goals: resident ASR/audio-tap integration, live translation, new model defaults,
model installer, cloud clients, Ask Nova, summaries, and discovery.

Reconciliation: M17 remains partial (resident ASR is unwired); M18 gains a VOD/episode
caption consumer; M19 remains provider-owned; M20/AI16 and AI19 remain missing. The
existing AI01–AI05 runtime/registry/governor foundations are reused.

## Delivered and validated

Implemented the contract in `commands/translation.rs`, the existing caption
session/storage repositories, and the player caption hooks/picker/overlay. The
source event and command result use Rust-generated `CaptionCue`/`CuesEvent` types.
SQLx offline metadata was generated against a synthetic combined schema.

- `just lint` and `just typecheck`: pass.
- `pnpm -C apps/desktop build`: pass (existing Vite bundle-size advisory).
- Focused frontend suite: 14 tests pass (batch limits, stale target/session/media,
  failures/retry, committed-only input, RTL, appearance settings, picker/live gating).
- Real SQLite tests exercise segment isolation, cache hits before inference,
  model revision misses, preserved timestamps, and no writes after failed,
  malformed or cancelled inference. The caption manager tests scope, cancellation
  and one-batch admission.
- `just test`: pass, including all Rust workspace tests and 182 frontend tests.
  The first sandboxed attempt could not bind existing artwork loopback fixtures;
  the approved run with loopback access passed.
- Independent read-only review found appearance-setting loss and first-render
  stale-media dialogue. Both were fixed, regression-tested, and re-reviewed with
  no remaining confirmed blockers.

No real-model inference or native-provider playback was qualified in this slice;
the configured local model root did not provide an available matching Opus layout.
Browser visual inspection was blocked by automatic approval review's usage limit.
The preview server was stopped and scratch preview files removed.

Next qualification: install/register the pinned direction models, then exercise
VOD and episode captions on device (direct and pivot routes, seek, track/target
switch, missing model/SSD disconnect, and playback contention). Live streaming
still depends on M17 audio capture. Ask Nova and discovery remain separate work.
