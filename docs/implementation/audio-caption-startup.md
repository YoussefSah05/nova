# Available audio tracks and caption startup choices

## Contract

Extend the existing bounded private audio probe with ordered audio ordinals and normalized
nullable languages; retain safe summary fields and generated IPC types. Remove fixed-language
menus and profile writes from movie/legacy details. A shared Audio selector lists only the
selected source's tracks, including duplicate/unknown labels and loading/error/retry states.
Selection is transient, keyed to the exact provider target, and never crosses versions.

Shared Play/Resume actions offer normal playback (captions off), source transcription, and
translated captions EN/FR/AR. Apply this to movie/legacy detail and series primary episode
actions. Separate translation targets from recorded audio. Pass optional launch options
through existing playback resolution, preserving legacy callers. Explicit choices override
profile defaults for this request, with a fresh player for each explicit launch. In-player
next episode/recovery retain caption choices and discard source-specific audio overrides.

Video begins immediately. Start source inference only after native resume position and chosen
audio track are confirmed. Explicit audio ordinal precedes the matching profile preference,
then player default. Invalid/stale choices fall back visibly; caption/model failure preserves
video and source captions, with existing explicit model controls/download flows. No migrations,
new AI pipeline, dubbing, automatic model activation or live captions.

Add focused tests for real track metadata, EN-only/duplicate/unknown menus, stale target changes,
startup propagation/default precedence/resume readiness, reopen and next episode continuity.
Run focused checks then repository lint/typecheck/tests, independent review, serial app bundling
and native UI validation; record limitations, update status. Commit only owned
files and fast-forward main locally, preserving preexisting user work; never push.

## Delivered behavior

The bounded audio-only probe now exports ordered audio ordinals with nullable normalized
language tags as generated Rust-owned DTOs. Detail Audio menus offer Auto and actual tracks;
duplicate/unknown labels preserve track identity. Canonical source cards select a version
without playing, and changing that source clears its audio choice. Shared Play/Resume
controls launch normal, source transcription or EN/FR/AR translated captions. The options
travel through canonical resolution; stale asynchronous resolutions cannot replace newer
launches. Each explicit request remounts the player even for the same title. Automatic next
and recovery reuse its current caption controls but discard the old audio ordinal.

Native startup applies a source-bound ordinal before profile/default audio, verifies its
language against actual native metadata, and waits for native acknowledgement. Invalid
choices, rejected commands and acknowledgement timeout keep video running with a visible
fallback. A settled startup gate remains settled across later manual audio changes so
transcription can retarget normally. An ordered coordinator query confirms the resume seek
and returns the actual clock before ASR activation. Unknown-duration sources without a
resume seek remain eligible. Failed seeks use the coordinator's existing bounded retries.

Disabled Whisper explains Settings → Models and Retry. Missing Whisper keeps the explicit
download action. Retry rechecks availability without a download or automatic model enable.
The existing translation hook and overlay preserve source captions on translation failure.
No migrations, new model pipeline, live AI captions or profile preference writes.

## Review and validation

Fresh independent review found and verified fixes for three startup issues: ASR racing the
coordinator resume seek; initial audio acknowledgement pinning captions to that track; and
canonical version cards starting normal playback before audio/caption choices. The reviewer
reran 29 focused tests and found no remaining confirmed actionable issues.

Focused regressions cover EN-only menus versus translation targets, duplicate/unknown tracks,
failed-probe Retry, version changes and stale responses, actual audio acknowledgements and
bounded fallback, resume confirmation and old tokens, disabled/missing model flows, profile
precedence, explicit same-title reopening, canonical request races and next-episode caption
continuity. The real four-track FR/AR/EN/untagged container probe passes with ordered metadata.

The initial full-suite sandbox run could not bind test loopback servers; rerun with the needed
local test access is recorded separately. Final full-suite/build/native results are recorded below.

Native GUI launch additionally exposed that media tools depended on the shell PATH. The
installed FFmpeg/ffprobe now resolve from the existing PATH followed by standard macOS
Homebrew locations, preserving authoritative explicit overrides. This is needed for the
requested Audio probe and transcription to work from a desktop launch; it installs nothing
and does not change system PATH or preferences.

The provider probe was initially blocked by automatic approval review because URLs can
contain credentials. The user explicitly authorized validation against configured sources;
URL-free direct results were one successful single-audio source and one timed-out source.
No URLs or credentials were printed or stored in repository artifacts. Native GUI loading,
probe failure/retry, source selection without playing, and separate EN/FR/AR caption choices
were inspected. An existing GuideGrid keyboard test was stabilized to await the intended
focus movement before Enter, without changing guide behavior.

Final `just test` passes: **746 Rust tests/doctests and 225 frontend tests**. `just lint`,
`just typecheck`, scoped Prettier/rustfmt and whitespace checks pass. The final debug Tauri
app bundle includes the frontend production build and locked native build.

After the approved restart, 4L's selected source displayed **Auto and EN only** in Audio;
choosing EN updated the per-play selector. Its Play menu offered source transcription and
EN/FR/AR translated captions separately. Launching Arabic translated captions immediately
started native video; the player confirmed Translation and Arabic selected while caption
status reported Extracting audio and the video clock advanced. A fresh same-title launch
resumed around 1:10 before extraction; the companion then showed generated source dialogue
starting at 1:10, with timed entries through 1:34 while translation prepared. Version changes and failed
provider metadata were also checked earlier without starting playback or fabricating audio.

This qualifies menu/startup wiring and selected-source native playback. Multi-track ordinal,
resume acknowledgement, model-failure fallback and automatic-next continuity are additionally
covered by focused tests. Full-film accuracy, every dialect, and measured UI frame rate are
outside this change's qualification; translation output requires its installed/enabled models.
