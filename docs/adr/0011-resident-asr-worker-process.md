# ADR 0011: Resident speech recognition in a worker process

Status: accepted, 2026-10-07.

## Context

Live captions need first text within about a second of speech. Today's batch path
starts a new `whisper-cli` process for every 30 s window, so the model loads again each
time. The AI22 benchmark (`docs/evaluations/ai22-2026-10-07.md`) picked whisper
large-v3-turbo q5_0 with a 1 s LocalAgreement-2 step, which needs the model kept loaded
across decodes.

Running whisper.cpp inside the app would keep it loaded, but it would break the
bootstrap rule that an ASR failure must not break playback (§37):
- whisper.cpp and ggml stop the whole process with `abort()` on a failed assertion;
- the first Core ML load blocks for 31–38 s while macOS compiles the encoder for the
  Neural Engine.

## Decision

whisper.cpp runs **resident in a worker process**: Nova's own executable, started with
`--asr-worker <model.bin>`. `main.rs` dispatches to `nova_transcribe::streaming::worker`
before Tauri starts.

**Protocol** (`streaming/protocol.rs`), over the worker's stdin and stdout:
- JSON lines, with raw f32 samples after each decode request;
- one answer per request, in order;
- the worker points fd 1 at stderr so whisper.cpp's printf output can never corrupt the
  protocol.

**Cold start:**
- The worker first loads the GGML weights through a link that has no Core ML encoder
  beside it. whisper-rs builds with `WHISPER_COREML_ALLOW_FALLBACK`, so whisper.cpp falls
  back to Metal, and the worker reports ready immediately.
- A background thread then loads the Core ML encoder. The worker switches to it between
  two decodes and frees the Metal copy.
- The Metal and Core ML paths have the same accuracy, so captions never wait for the
  compile.

**Lifecycle:**
- The worker belongs to the whisper backend in `ModelRuntimeManager`, which starts it on
  the first `ModelLease::speech_engine()`.
- The worker dies when the model is unloaded or evicted, so leases, admission and the
  resident budget all apply unchanged.
- The whisper model now counts against the resident budget.
- `prewarm_speech` starts the worker at interactive priority when the user signals
  intent. It is never triggered speculatively.

**Failure handling:**
- A worker that dies, hangs past a deadline (90 s to become ready, 20 s per decode) or
  sends malformed output is killed.
- The current step fails, and the next step starts a fresh worker.
- Playback is never affected.

**Streaming policy:** `StreamingSession` runs the benchmarked LocalAgreement-2 policy:
- a 1 s step and a 15 s buffer, trimmed at committed sentence ends;
- a prompt built only from committed text whose audio has left the buffer;
- the repetition-loop guard;
- the detected language locked once the buffer reaches 3 s.

Audio arrives through a bounded `PcmQueue`. When the queue is full, the oldest audio is
dropped and counted. Provisional updates are ephemeral; only committed updates are final.

**Batch path:** `whisper-cli` keeps serving batch (VOD window) transcription until the
streaming caption pipeline (M17-b) replaces it.

## Alternatives rejected

- **whisper-rs in the app process:** a whisper.cpp abort or a blocked Core ML compile
  would take playback down with it.
- **`whisper-server` over HTTP:**
  - every step would pay for a WAV encode and an HTTP round trip;
  - Nova would lose control of word reconstruction (byte-level joining of tokens, which
    Arabic needs);
  - the server is an external binary that Nova does not bundle.
- **A separately bundled sidecar binary:** it would need its own build, signing and
  bundling for code Nova already ships. Running Nova's own executable as the worker gives
  the same isolation without that.
- **Apple SpeechTranscriber:** it has no Arabic. It remains an option for English and
  French later.

## Consequences

- **Measured with the `live_asr` example** (French FLEURS, 50 s, real-time feed, model on
  the external SSD):
  - ready on Metal 7.6 s after spawn, cold from the SSD (1.5 s with the file cached);
  - committed text trailing speech by about 1 s per step;
  - the Core ML switch mid-stream at about 39 s, without a gap;
  - no dropped audio.
- **External libraries:** the worker reads the model path directly. That is valid
  because Nova is not sandboxed. A sandboxed build would have to pass the
  security-scoped access to the worker, or pass the file itself.
- **Memory:** while Core ML compiles, both encoders are loaded (about 1.8 GB for turbo
  q5) until the switch. The steady state is about 900 MB.
- **Numbering:** the playback audio tap moves to ADR 0012.
