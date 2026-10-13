# ADR 0014: Caption word timing and cold start

Status: accepted, 2026-10-13.

## Context

Two problems with source captions on movies and episodes:

1. **Timing.** Caption times came from Whisper. Nova asked `whisper-cli` for DTW token times, but
   whisper.cpp 1.8 turns flash attention on by default, and flash attention silently disables DTW:
   every `t_dtw` was `-1`, and the parser read the interpolated `offsets` instead. On a 779-word
   reference set, word starts landed a mean **398 ms** from the reference and only 26% within
   100 ms. Whisper also invents text over music and silence ("Thank you.", subtitle credits).
2. **Cold start.** Every 30 s window started a new `whisper-cli` process, which loads the model
   again. On a first run (or after macOS evicts its compiled-model cache) the Core ML encoder
   compiles before anything is decoded: the first caption took **32 s** in a reproduced cold run.
   ADR 0011 had built a resident worker that avoids this, but only live captions were planned to
   use it.

## Research: what production systems do

- **Word timing.** The common production recipe is WhisperX (Bain et al., Interspeech 2023):
  voice activity detection → Whisper → forced alignment of Whisper's text with a wav2vec2 model
  trained with CTC on characters. Whisper decides *what* was said; the CTC model, whose outputs
  are tied to 20 ms audio frames, decides *when*. `ctc-forced-aligner` packages the same idea
  around one multilingual model, MMS-300M fine-tuned for alignment. Comparisons with hand-aligned
  references (Rousso et al., Interspeech 2024, "Tradition or Innovation") find classic HMM
  aligners still slightly more precise, but they need per-language pronunciation dictionaries,
  which Nova cannot ship for dialectal Arabic.
- **VAD.** Silero VAD is the standard open detector (whisper.cpp, faster-whisper and WhisperX all
  use it or Pyannote). Its job here is to keep Whisper away from non-speech.
- **Cold start.** Serving stacks keep models resident and warm (pre-downloaded weights, warm-up
  requests, reused recognizers); media servers that generate subtitles (Bazarr + Subgen,
  Jellyfin plugins) do it ahead of time when media is added or playback starts. Streaming ASR
  research trades smaller first chunks against accuracy. Nova combines the three: resident model,
  intent-driven warm-up, and a short first chunk.

## Decision

### Timing: VAD + Whisper + CTC alignment, inside the ASR worker

- Two new manifest models, installed from Settings → Models → **Caption timing**, pinned by
  revision, size and SHA-256 like the translation models:
  - `mms-300m-forced-aligner`: `onnx-community/mms-300m-1130-forced-aligner-ONNX`, uint8 ONNX,
    317 MB, CC BY-NC 4.0 (labelled non-commercial).
  - `silero-vad-v5`: `onnx-community/silero-vad`, 2 MB, MIT.
- New crate `nova-align`: Silero VAD, CTC emissions, an exact Viterbi forced aligner over the
  blank-augmented token lattice, romanization into the aligner's a–z vocabulary (accents removed;
  Arabic transliterated letter by letter, as `uroman` does for the model's training data), and
  `refine`, which combines them:
  1. align every word of the window in **one joint pass** (word order, not Whisper's segment
     bounds, places each word: those bounds can sit a second off);
  2. accept a word when its mean letter log-probability is at least −4, interpolate the rest
     between confident neighbours, clamp everything to the analysed audio;
  3. drop a segment only when none of its confidently aligned words lands on detected speech and
     its own span holds under 0.15 s of speech (the VAD is the independent witness; the aligner
     always places every word somewhere, so placement alone proves nothing);
  4. keep cues from overlapping.
- The models run **in the ASR worker process**, beside whisper.cpp, not in the app:
  - the resident budget is two models and one model operation at a time; a third in-app model
    would be evicted between windows by Whisper and Opus-MT;
  - the worker already holds the window's PCM;
  - a crash in ONNX Runtime ends the worker, never playback (ADR 0011's rule).
  The app leases the two models through a non-resident `CompanionAdapter` that verifies their
  checksums and hands their directories to the worker in each request (`DecodeRequest.timing`).
  The lease keeps them from being removed or replaced mid-decode. The worker loads each on first
  use and reports what the stage did (`Decoded.timing`).
- A window in which the VAD hears no speech is never decoded.
- DTW is now really on, in both paths: the worker sets the alignment-head preset from the model
  name with flash attention off (whisper-rs's default), and `whisper-cli` gets
  `--no-flash-attn`. DTW gives each token one instant; the parser now reads it.
- Without the timing models, captions keep Whisper DTW times (fixed above), and a VAD alone still
  snaps cue edges to speech.

### Cold start: resident worker, warm-up on intent, short first window, prepare and adopt

- **VOD captions decode on the resident worker** (the M17-b step ADR 0011 deferred). It starts on
  Metal in about 0.3 s with the weights cached (7.6 s cold from an SSD, ADR 0011) and loads the
  Core ML encoder in the background. `whisper-cli` remains the fallback when the worker cannot
  start.
- **Windows are 27 s of new audio + 3 s of lead-in = 30 s**, Whisper's native context. The old
  33 s windows made whisper.cpp decode a second 3 s chunk, which is slow and prone to invented
  text.
- **The first window after a start is 8 s of new audio** (11 s with the lead-in), so the first
  caption waits for less audio to download and decode.
- **Warm-up on intent** (`prewarm_captions`): opening a title's playback menu, or the player
  opening with captions on. The player's caption session itself waits for the audio-track and
  resume-seek confirmations, so the worker is ready by then.
- **Prepare and adopt** (`prepare_captions` / `cancel_caption_prepare`): when a title page opens
  for a profile whose captions start on, or its playback menu opens, Nova transcribes the opening
  from the resume point (45 s ahead) at interactive priority. When the player starts a caption
  session for the same profile, media and audio track, it **adopts** the prepared session: the
  session id its events carry switches to the player's, its range and priority become the
  player's, and the cues already committed are replayed. A prepared session never downloads a
  model, never replaces a player's session, and stops when its page closes unplayed. Prepared
  windows are persisted like any others, so even an unadopted prepare makes the next open
  instant.

This amends ADR 0011's "never speculatively" rule for warm-up: hovering still does nothing; a
deliberate click (opening a title with captions-on as the profile default, or opening the
playback menu) counts as intent.

## Measurements

Full method and per-window data: [`docs/evaluations/caption-timing-2026-10-13.md`](../evaluations/caption-timing-2026-10-13.md).

| Word starts vs. reference (779 words) | Mean error | Within 100 ms |
|---|---|---|
| Before (interpolated offsets) | 398 ms | 26% |
| Whisper DTW (flash attention off) | 215 ms | 10% |
| DTW + VAD + CTC alignment | **50 ms** | **92.5%** |

| First caption on 2 min of real speech | Old: CLI per window | New: resident worker |
|---|---|---|
| Core ML encoder never compiled | 32.1 s | **2.1 s** |
| Everything cached | 2.6 s | 2.4 s |

The timing stage costs about 2.3 s of CPU per 30 s window (uint8 aligner, four threads): a
window takes ~4 s for 27 s of new audio, about 6× real time, inside the 90 s prefetch.

## Alternatives rejected

- **Aligner in the app process.** Residency thrash (see above) and a native-crash path into
  playback.
- **whisper.cpp's own `--vad`.** It remaps segment offsets but leaves DTW token times on the
  VAD-compressed timeline, corrupting word timing. Nova's VAD runs on the original timeline.
- **Per-segment alignment (WhisperX).** Measured: segments that Whisper placed in a pause lost
  their words. The joint pass placed all of them.
- **Per-language wav2vec2 models.** Three models (~1.2 GB each before quantization) instead of one,
  and no ONNX exports at pinned revisions for French and Arabic.
- **The q4 aligner.** Same accuracy, 60% slower on CPU than uint8.
- **A tiny Whisper model for the first window.** It would give the first captions a different
  quality and a second model to hold; the short first window gets most of the latency win
  without that.

## Consequences

- Alignment uses ~450 MB more in the worker while captions run; the whisper model's resident
  estimate covers the worker as a whole.
- The aligner is CC BY-NC: acceptable while Nova is non-commercial, and labelled in Settings.
- English accuracy is measured; French and Arabic are checked on real FLEURS speech (all words
  aligned, plausible boundaries) but have no reference boundaries yet.
- Live captions keep their current path; the timing stage is available to them through the same
  request field.
