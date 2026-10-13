# Whisper configuration

How `crates/transcribe/` runs whisper.cpp, and how caption timing is refined around it.

Captions decode on the **resident ASR worker** (ADR 0011, ADR 0014): one process keeps the model
loaded, starts on Metal at once and switches to the Core ML encoder when it has compiled.
`whisper-cli` is the fallback when the worker cannot start.

## Model

The default is **large-v3-turbo, q5_0** (~500 MB): close to large-v3 quality at a third of the
size. On Apple Silicon the encoder runs through Core ML; the first run compiles the Core ML model
(30–120 s) and later runs reuse it. The resident worker does not wait for that compile: it decodes
on Metal meanwhile. Model paths are resolved from the app-data directory, never
hard-coded.

## `whisper-cli` arguments

| Argument | Why |
|---|---|
| `-m <model> -f <window.wav>` | Model and input window |
| `--output-json-full --output-file <base>` | Segments **and** per-token offsets: the source of word timing |
| `--dtw <preset> --no-flash-attn` | DTW token times from the cross-attention heads (e.g. `large.v3.turbo`). whisper.cpp 1.8 enables flash attention by default, which **silently disables DTW** (`t_dtw` is `-1` on every token): the flag pair is required, and the parser reads `t_dtw` |
| `--language <tag>` | When the spoken language is known; `auto` otherwise |
| `--prompt <text>` | The previous window's tail, for context across window boundaries |
| `--max-len <N> --split-on-word` | Caption-sized segments split at word boundaries |
| `--max-context 0` | Arabic: reduces hallucination across silences (the previous-window prompt is also omitted) |

Transcription is whisper.cpp's default task. There is no `--task` flag, and `--translate` (English
only) is never used: translation runs in `crates/translate`.

The pinned CLI (1.8.3) does not support `--no-context` / `-nc`: those flags print help, exit
successfully and write no output.

The worker sets the same DTW preset when it creates its whisper context (flash attention off) and
reads `t_dtw` per token.

## Windows

27 s of new audio plus a 3 s lead-in: exactly Whisper's 30 s context, so whisper.cpp never decodes
a short second chunk. The first window after a start is 8 s (+3 s), so the first caption waits for
less audio. Segments ending within 1.5 s of a cut are re-read by the next window with the
previous tail as the prompt.

## Caption timing (VAD + forced alignment)

With the Caption timing models installed (Settings → Models), every worker decode runs:

1. **Silero VAD v5** on the window: no speech → Whisper is not run at all.
2. **Whisper** (DTW) for the text.
3. **MMS-300M CTC forced alignment** (`crates/align`): all words of the window aligned jointly
   to 20 ms frames; uncertain words interpolated; segments with no confidently aligned word on
   detected speech dropped as hallucinations.

whisper.cpp's own `--vad` is still not used: it remaps segment offsets but leaves DTW token
offsets on the VAD-compressed timeline. Nova's VAD runs on the original timeline.
Measurements: [`docs/evaluations/caption-timing-2026-10-13.md`](../evaluations/caption-timing-2026-10-13.md).

## Captions

- Cues are capped at two lines of about 70 characters.
- Very short segments are merged with their neighbour.
- Leading and trailing whitespace is trimmed.
