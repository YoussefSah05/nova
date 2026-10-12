# Whisper configuration

How `crates/transcribe/` runs whisper.cpp.

## Model

The default is **large-v3-turbo, q5_0** (~500 MB): close to large-v3 quality at a third of the
size. On Apple Silicon the encoder runs through Core ML; the first run compiles the Core ML model
(30–120 s) and later runs reuse it. Model paths are resolved from the app-data directory, never
hard-coded.

## `whisper-cli` arguments

| Argument | Why |
|---|---|
| `-m <model> -f <window.wav>` | Model and input window |
| `--output-json-full --output-file <base>` | Segments **and** per-token offsets: the source of word timing |
| `--dtw <preset>` | Always: DTW-aligned token timestamps from the cross-attention heads (e.g. `large.v3.turbo`); without it word timing drifts |
| `--language <tag>` | When the spoken language is known; `auto` otherwise |
| `--prompt <text>` | The previous window's tail, for context across window boundaries |
| `--max-len <N> --split-on-word` | Caption-sized segments split at word boundaries |
| `--max-context 0` | Arabic: reduces hallucination across silences (the previous-window prompt is also omitted) |

Transcription is whisper.cpp's default task. There is no `--task` flag, and `--translate` (English
only) is never used: translation runs in `crates/translate`.

The pinned CLI (1.8.3) does not support `--no-context` / `-nc`: those flags print help, exit
successfully and write no output.

## Voice activity detection: not used

whisper.cpp's VAD remaps segment offsets to the original timeline but leaves DTW token offsets on
the VAD-compressed timeline, corrupting word timing. Nova therefore skips VAD and segments on
whisper's natural, pause-aligned boundaries in the caption session.

## Captions

- Cues are capped at two lines of about 70 characters.
- Very short segments are merged with their neighbour.
- Leading and trailing whitespace is trimmed.
