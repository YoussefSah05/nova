# AI model evaluation: rubric (AI22)

Date: 2026-10-07. Status: **harness built; first full matrix pending.**

Nova's local speech recognition (ASR) and translation engines are chosen by
measurement. The harness is `tools/ai-bench`; see its README for how to run it. Results
go to `docs/evaluations/` as Markdown summaries. Raw JSON stays with the data, outside
the repo.

## Scope

**Requirements:**
- Spoken languages: English, French and Arabic (including dialects).
- Translation: any pair among English, French and Arabic.
- macOS 26 or later, on Apple Silicon.
- Models may live on an external SSD.
- Non-commercial licences are allowed for now, but are flagged.

**Data:**
- ASR: FLEURS test, one recording per distinct sentence.
  - Offline: 40 clips per language.
  - Streaming: three ~90 s programmes per language.
- Translation: FLORES-200 devtest, the first 100 sentences per direction.
- Real IPTV clips (dialects, music, crosstalk) are a private, uncommitted supplement.

## Hard gates

A candidate must pass every gate for a language or direction to become its default.

| Gate | Threshold | Why |
|---|---|---|
| First text, p50 (warm) | ≤ 1.0 s | Captions should appear about as soon as speech starts |
| Committed lag, p95 | ≤ 3.5 s | Final text should trail the speaker by at most a few seconds |
| Offline RTF | ≤ 0.3 | Headroom for seeks, catch-up and other work |
| Streaming keeps up | RTF < 1.0 | A live engine must never fall behind |
| Arabic WER | ≤ 1.25× the best Arabic candidate | Arabic quality must not be sacrificed for speed |
| Peak memory | ≤ 2.5 GB | Must coexist with playback and the rest of the app |
| Translation latency per segment, p95 | ≤ 400 ms | Translated subtitles keep pace with committed text |

These thresholds are encoded in `tools/ai-bench/src/report.rs`. Changing one is a rubric
change: it must be recorded here, with the reason.

## Choosing among candidates that pass

- **ASR:** lowest streaming WER, then lowest committed lag p95, then lowest load time.
  - A different default per language is allowed. For example, Apple's engine supports
    no Arabic, so Arabic keeps a Whisper model.
- **Translation:** highest chrF++, then lowest p95 latency.
  - A direction without a passing direct model may pivot through English, if the
    pivot's quality and combined latency still pass.
- **Licences:** a non-commercial model (such as NLLB) may be the default only while
  Nova is non-commercial, and Settings must label it.

Defaults are written into the model manifest (slice 2), and the change must cite the
summary file it rests on.

## What the harness already showed

These are early, partial runs (English only, before the full matrix):
- Whisper turbo q5 with Core ML, at a 1 s step on English FLEURS:
  - offline WER 0.065;
  - streaming WER 0.070, after two fixes to the streaming policy;
  - first text 1.48 s;
  - committed lag p50 1.87 s, p95 5.3 s;
  - peak memory 755 MB.

  It fails the first-text and lag gates. Smaller models and a 0.5 s step are in the
  matrix to address that.
- **A cold Core ML load takes 31–38 s**: macOS compiles the encoder for the Neural
  Engine, and it discards that cache from time to time. A warm load takes 0.4 s. The
  product must therefore compile and pre-warm in the background, or start on the Metal
  path and switch once Core ML is ready. A user must never wait for this compile.
- **Apple SpeechTranscriber does not support Arabic** on this system. It supports
  English, French and about 25 other locales. In fast mode, English text first appears
  in about 1.0 s.
- **Opus-MT English→French** in-process on the app's ONNX Runtime: chrF++ 67.4,
  BLEU 47.2, 175 ms p50 and 298 ms p95 per sentence.
- **Apple Translation** supports all six directions, but needs the language packs
  installed by the user.
