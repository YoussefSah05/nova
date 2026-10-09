# Whisper CLI installation and discovery repair

## Implementation contract

The player uses the existing cancellable `whisper-cli` driver, not the compiled resident
speech worker. The reported spawn error is reproducible: no CLI exists on PATH, no
NOVA_WHISPER_BIN override is set, and neither Homebrew Whisper formula is installed.
Existing Whisper model files are retained. No conflicting system installation was found.

Reuse the current driver, model registry/leases, and native setup. Build an official,
checksum-pinned whisper.cpp 1.8.3 CLI matching the vendored resident backend, with static
libraries, embedded Metal shaders, Core ML and Metal fallback. Install only Nova's runtime
under its application data folder; replace that binary only after a successful build/check.

Add a repeatable `just install-whisper` command and explicit setup integration. Nova startup
registers its managed runtime directory. Resolve explicit override, managed CLI, executable
PATH candidates and known Homebrew CLI locations to absolute paths. Reject non-executable
files; remove the ambiguous generic `main` fallback. No runtime download/install or shell
profile changes. Errors remain useful without exposing paths/model text.

No schema/model/default/resource-policy changes. Tests cover resolution precedence,
restricted GUI PATH, executable permission and cancellation. Validate real transcription
using the installed weights and public JFK sample through the same Rust driver, preserving
JSON-full, DTW, max-length/split-on-word, prompt and language normalization. Run repository
checks and obtain independent review. Signed app distribution remains a separate gate.

## Delivered and measured

The private CLI was built and installed successfully from official v1.8.3 source with its
pinned SHA-256. Its dependency inspection lists only system frameworks/libraries, including
Core ML and Metal. No old global Whisper install was present; existing Rust worker builds
were preserved, as were all model weights and profiles. Private installation needs no sudo,
package-manager reinstall, shell profile edit or environment override.

Both real-device runs used the existing internal large-v3-turbo q5_0 GGML weights and the
official release's public JFK WAV, through `transcribe_wav` with the player's JSON-full,
DTW and 60-character word-split arguments, including ISO-639-2 `eng` normalization.
The first run took **46.81 seconds** to return caption data; the warm repeat took **1.55
seconds**. Both produced two source segments and 22 finite, ordered word timestamp spans
within the sample's time range, and recognized the reference speech. This measures local
sample startup/inference/parsing, not a provider's extraction/network delay or every
language's recognition/alignment accuracy.

An opt-in ignored test retains this qualification path (`NOVA_TEST_WHISPER_ROOT` plus
`NOVA_TEST_WHISPER_WAV`, filtered test `installed_cli_transcribes_real_speech_with_the_player_driver`).
Ordinary tests exercise restricted PATH, explicit override, managed precedence, permissions,
known locations and rejection of unrelated/legacy executables without touching global env.
Independent review found no blockers. A running older app needs restarting to register the
private runtime location. Live captions and signed app distribution are unchanged.

## Validation

- `just lint` and `just typecheck` passed.
- `just test` passed: 730 Rust tests/doctests and 187 frontend tests; nine opt-in Rust tests
  were ignored by the ordinary suite. The real-model test above passed separately twice.
- `SQLX_OFFLINE=true cargo build -p nova-desktop --locked` passed.
- Installer/bootstrap shell syntax checks and native CLI dependency inspection passed.
- Independent review found no actionable issues in installation, discovery or lifecycle.

Upstream build references: [official v1.8.3 Core ML instructions](https://github.com/ggml-org/whisper.cpp/blob/v1.8.3/README.md#core-ml-support).
