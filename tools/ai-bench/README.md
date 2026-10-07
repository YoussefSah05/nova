# nova-ai-bench: local ASR and translation benchmark (AI22)

This harness chooses Nova's speech recognition (ASR) and translation engines from
measurements, not guesses. The decision rules are in
[`docs/implementation/ai-model-evaluation.md`](../../docs/implementation/ai-model-evaluation.md),
and results go to `docs/evaluations/`.

It is a **standalone Cargo workspace**, excluded from the app workspace. It compiles
whisper.cpp with Metal and Core ML, which is a heavy C++ build that would otherwise slow
every app build and CI run. It pins the same `ort` / `ort-sys` as the app, so translation
is measured on the app's ONNX Runtime with the pure-Rust `tokenizers` crate (no protobuf).

## What it measures

**Speech recognition**, per language: English, French, Arabic (FLEURS test).

- **Offline:** each utterance decoded alone. This gives WER/CER (normalized, with Arabic
  diacritics and letter variants unified), speed (RTF: compute time ÷ audio duration),
  and language detection.
- **Streaming:** utterances joined into ~90 s "programmes", as with continuous broadcast
  audio, and transcribed live. The metrics are:
  - the time until text first appears;
  - per-word *shown lag* (first displayed minus spoken end) and *committed lag*
    (finalized minus spoken end), p50 and p95;
  - the rewrite rate (provisional words later replaced);
  - streaming RTF and WER.
- **Load time and peak memory.**

The engines:

- **whisper.cpp, in-process and resident** (`whisper-rs`, Metal; Core ML encoder with
  `--features coreml`).
  - Streaming uses the **LocalAgreement-2** policy (Macháček et al., 2023): re-decode
    the buffer every step, and commit the word prefix two consecutive hypotheses agree
    on.
  - Already-committed audio is matched by text alignment, not timestamps.
  - Streaming runs on a **virtual clock**: a step starts once its audio would have
    arrived and the engine is free, plus its measured compute time. Latencies are what
    a live viewer would see, but the run takes a fraction of real time.
- **Apple SpeechTranscriber** (macOS 26), through `swift-helper/`, out of process.
  - Fed in real time, since the analyzer's own pacing is part of what is measured.
  - Per-word timings come from the result's audio time ranges.
  - `--fast` uses Apple's `.fastResults` low-latency mode. Arabic is not supported by
    Apple.

**Translation**, per direction among EN, FR and AR (FLORES-200 devtest).

- **Metrics:** chrF++ and BLEU (verified equal to sacreBLEU 2.5.1), and per-segment
  latency p50/p95 after a warm-up.
- **Engines:**
  - **Opus-MT/Marian** ONNX, one model per direction, with an English pivot when no
    direct model exists (FR↔AR);
  - **NLLB-200 distilled 600M** ONNX (CC-BY-NC, flagged);
  - **TranslateGemma 4B** GGUF through a loopback `llama-server`, with the previous
    lines as context;
  - **Apple Translation** through the helper. This needs the language pairs installed:
    System Settings → General → Language & Region → Translation Languages.

## Usage

```sh
# Models and data (~9 GB) go OUTSIDE the repo, ideally on the external model SSD.
# Every file is checked against manifests/*.json before use.
python3 tools/ai-bench/fetch.py /Volumes/Lexar/nova-ai/bench

cd tools/ai-bench
cargo build --release                                        # Metal
CARGO_TARGET_DIR=target-coreml cargo build --release --features coreml
(cd swift-helper && swift build -c release)                  # Apple engines

B=/Volumes/Lexar/nova-ai/bench
# Speech: one candidate per run, all three languages.
./target-coreml/release/nova-ai-bench asr --name whisper-turbo-q5-coreml \
  --model $B/models/whisper/ggml-large-v3-turbo-q5_0.bin \
  --data $B/asr/fleurs --langs en_us,fr_fr,ar_eg --no-fallback --out turbo-coreml.json
./target/release/nova-ai-bench asr --engine apple --fast --name apple-fast \
  --helper swift-helper/.build/release/nova-apple-helper \
  --data $B/asr/fleurs --langs en_us,fr_fr --out apple-fast.json

# Translation: one engine per run, all directions.
./target/release/nova-ai-bench mt --name opus --engine opus --models $B/models/opus \
  --pairs en-fr,fr-en,en-ar,ar-en,fr-ar,ar-fr --flores $B/mt/flores200/devtest --out opus.json

# One Markdown summary with every gate evaluated.
./target/release/nova-ai-bench report *.json --out summary.md
```

Real IPTV clips (news, sports, film, Arabic dialects) belong in a private dataset folder
in the same layout (`<lang>/manifest.jsonl` with `{id, audio, text}`). They are never
committed.

## Caveats

- FLEURS is read speech, and its Arabic is mostly Modern Standard Arabic. Broadcast
  audio and dialects need the private clips before any final choice for Arabic.
- The first Core ML load compiles the model for the Neural Engine. This measured 38 s
  for turbo, then 0.4 s warm. Report warm runs, and record the cold load separately.
- The Apple engines download their language assets on first use. Run once to warm up
  before measuring.
