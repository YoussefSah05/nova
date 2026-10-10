# Caption reliability and interface polish

## Implementation contract

Reuse the cancellable Whisper driver, bounded VOD windows, transcript repository,
resource admission, shared screen/motion system and detail frames. Preserve existing
models, profiles and uncommitted user work; no migration or replacement AI pipeline.

Confirmed failures: the pinned CLI rejects Arabic's `--no-context` flag but exits zero;
unsupported two-letter language tags do the same. FFmpeg may return a header-only WAV
after a seek beyond EOF. Sessions currently persist failed windows as covered, losing
dialogue. Fix supported language/context arguments and retain useful bounded diagnostics;
validate decoded audio, distinguish EOF from failures, bound retries, and never mark failed
speech/extraction work as successful coverage. Clamp finite final windows and preserve
cancellation/reaping, DTW and selected audio-track behavior. Test late seeks, empty audio,
language variants, transient failure and recovery with real local media where feasible.

For home scrolling, remove the moving full-width backdrop blur, reduce small image decode
sizes, and extend scroll-hover protection to editorial cards and horizontal rails while
keeping clicks usable. Eliminate redundant active-section transitions; polish shared button
feedback with visible focus and reduced-motion support. Preserve navigation state and
avoid per-frame React rendering or extra animation on virtualized rows.

Expose compact audio-language badges from real stream tags on active detail pages. A
bounded private backend probe returns only safe generated metadata, with nonfatal loading,
unknown and retry states. Cache frontend results by profile/media target; never probe every
home card/episode. Movie badges refer to Nova's selected version; series badges explicitly
refer to the next/resume episode. Do not confuse original language, subtitles or provider
MULTI labels with confirmed audio languages. Playback and page rendering remain immediate.

Run focused regressions, repository lint/typecheck/test and native/frontend builds; perform
independent review and update implementation status. Record measured evidence separately
from unqualified provider/language accuracy or WebKit frame-rate claims.

## Delivered behavior

- Whisper 1.8.3 receives supported `--max-context 0` for Arabic, normalized supported
  language tags, no Arabic prompt carryover, and appended JSON output suffixes. Missing
  output includes bounded actionable diagnostics. DTW word timing remains enabled.
- Caption windows stay bounded to 30 seconds plus 3 seconds of overlap, including late
  film positions. Decoded PCM duration determines EOF; header-only, incomplete and failed
  windows receive three cancellable attempts and remain retryable when exhausted. Silent
  speech output never establishes EOF or locks auto-detection to a guessed language.
  New transcript rows wait for confirmed speech, then persist prior silence coverage.
  Older empty rows can repair their language/artifact metadata without relabeling text.
- Home uses a cheaper static tinted header, smaller editorial image variants, and scroll
  intent protection across vertical and horizontal surfaces. Hover decoration, portals and
  speculative prefetch pause while scrolling; clicks, keyboard focus and reduced motion
  remain available. Shared buttons respond quickly and redundant root navigation is skipped.
- Movie, legacy VOD and series details show compact real audio tags from a private bounded
  ffprobe request. Movie metadata follows the selected version; series metadata labels the
  next/resume episode. Missing tags and probe failures show honest nonfatal states with retry.
  Provider labels, original language and subtitles are not audio-track evidence.

## Validation evidence and limits

The private installed official CLI and existing large-v3-turbo q5_0 model transcribed real
public French and Arabic FLEURS samples through Nova's Rust driver. French yielded two
segments / 19 timed words in 39.90 seconds on the first invocation; Arabic yielded three
segments / 15 timed words in 1.57 seconds on the next invocation. Tests check nonempty
speech, finite ordered segments, timed words and overlap with reference text; they do not
establish corpus-wide word error rate, exact synchronization or a universal startup SLA.

The opt-in two-hour synthetic audio test passed: FFmpeg decoded a bounded late window
and detected empty audio beyond EOF (0.10 seconds test time). This qualifies late seeking
and memory bounds, not two hours of continuous provider transcription. A real four-track
container probe returned FR/AR/EN and one untagged track (0.05 seconds test time).

FLEURS source: [official Google dataset](https://huggingface.co/datasets/google/fleurs),
CC BY 4.0. Validation-only WAV/reference files remain in ignored `target/validation`.
French WAV SHA-256: `c834367fd212902124bdd0e0f6c1f16b5561af6386600f145e5aec50008b298d`.
Arabic WAV SHA-256: `40d979a6239fc08951f387c671fb5fc46fffdc5df823ed1dbdb809bc75e26620`.
CLI behavior was checked against [pinned upstream source](https://github.com/ggml-org/whisper.cpp/blob/v1.8.3/examples/cli/cli.cpp);
stream-tag probing uses [official ffprobe fields](https://ffmpeg.org/ffprobe.html).

Independent final review found no remaining confirmed blockers; the guarded empty-cache
repair regression passed independently. SQLx metadata was generated against a synthetic
current schema, with no user database or migration changes. Full provider films, dialect
coverage, translation model inference and WebKit frame-rate benchmarking remain unqualified.

The rebuilt debug app bundle was opened after the user-approved restart. Native home
scrolling, navigation to movie details, loading feedback and a provider-backed EN badge
for 4L's selected version were verified visually. This is interaction/visual validation,
not a measured frame-rate claim. The first concurrent build/test run hit stale Tauri
artifacts during rustdoc; final test validation runs after bundling has finished.

Final validation: `just lint`, `just typecheck`, `just test` (743 Rust tests/doctests,
205 frontend tests), frontend production build, locked native build, debug app bundling,
`cargo fmt --all -- --check`, and scoped whitespace checks pass.
Twelve opt-in Rust tests remain ignored in the normal suite; the three targeted real
speech/late-media/audio-tag checks described above were run explicitly and passed.
Only this change's files are committed; existing user edits and untracked work are preserved.
