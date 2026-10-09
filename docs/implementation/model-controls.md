# Model controls and caption readiness

## Implementation contract

Reuse the bundled manifest, local registry, shared lease runtime, app_settings table,
Settings Models page, and caption-session status events.

Missing behavior: show every supported model (including all four translation directions),
effective internal/external availability, enabled state, and individual runtime activity.
Enable/disable is app-wide and persistent; enabling permits lazy use, without loading or
downloading. Disabling refuses active leases/loads, unloads idle backends, and blocks new
acquisitions. Existing cached data is preserved; cached translations may be reused without inference. Explicit unload only releases memory.

Persistence: two checked queries in a model-preferences repository using app_settings;
no migration. Runtime entry locking serializes preference changes with final lease creation.
Generated IPC DTOs contain no paths or bookmarks. No extra model processes or polling loads.

UX: Settings gains model controls and honest readiness/timing guidance. The player reports
audio extraction versus speech recognition/alignment, elapsed preparation time, and explains
windowed generation. Installed, enabled, prepared, and working remain distinct concepts.

Validation: persistence/defaults, disabled acquisition and busy protection, status precedence,
frontend controls and caption stages, then lint/typecheck/full tests and independent review.

Non-goals: changing ASR engines/windowing, live captions, new installers, automatic downloads,
quality benchmarking, or claiming real-device latency/alignment qualification from unit tests.

## Delivered and qualification

Settings lists all six manifest models with effective internal/external availability, persistent
enable/disable controls, and explicit idle memory release. Busy loads/leases cannot be disabled.
The semantic-search capability includes enabled state; installation badges describe files rather
than promising readiness. Whisper's prepared path adapter is distinguished from loaded weights.

Caption status events report preparation, extraction, model waiting and recognition/alignment.
The player measures time in each stage and first nonempty cue-batch readiness using its monotonic
clock. First-batch readiness is not time to the first visible caption: the audio may be silent or
the supplied cues may be ahead of the playhead. Recovered-window failures clear active phases.
Media/track/stop changes hide prior status immediately. No inference polling or extra workers.

Current VOD captions use `whisper-cli` on 30-second windows, 3-second overlap, selected audio track,
DTW token/word timestamps shifted back to media time, leading-word trimming, and a trailing guard
before committing segments. This is not the resident streaming engine evaluated in AI22.
Translation preserves committed source segment start/end; it does not align translated words to
individual spoken words, identify speakers, or guarantee transcription/translation correctness.

The [AI22 evaluation](../evaluations/ai22-2026-10-07.md) measured warm offline Whisper WER of
0.065 English / 0.075 French / 0.075 Arabic on 40 FLEURS clips per language. This describes read
speech recognition, not caption synchronization or accuracy on movies. The streaming engine's
fast provisional first text is not a guarantee for this windowed player. Cold Core ML compilation
was 31–38 seconds in the benchmark; current player startup latency remains unmeasured. A 30-second
audio window need not take 30 seconds of wall time to extract from VOD.

Warm Opus-MT translation measured p95 298 ms en→fr and 328 ms en→ar per sentence, once source text
was ready. Batch scheduling/loading adds time. Reverse directions are provisional; fr↔ar requires
two English-pivot hops. These are engine measurements, not end-to-end player latency.

Local read-only inspection found internal Whisper weights and a populated Core ML encoder; the
four internal Opus directories were absent. No external library or real inference was qualified
in this slice. Structural availability never proves successful inference. Native playback timing
and real speech alignment remain device-validation gates.

Validation: `just lint`, `just typecheck`, `just test` (728 Rust tests/doctests, 187 frontend
tests), frontend production build, and independent review pass. Review fixes cover offline
external fallback, stage-clock reset, disabled semantic capability, installation wording, and
clearing phases after recovered-window failures. External folder selection also refreshes model
and semantic capability queries, and its guidance includes the translation library layout.
