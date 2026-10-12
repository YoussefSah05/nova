# M18 translation model installer

## Implementation contract

Reuse the bundled manifest (pinned files, sha256, sizes), `attest`/`is_attested`, the runtime's
`mutate` lock, the shared `model:download:progress` event and the Settings Models page.

Missing behavior: the four Opus-MT directions had pins but no way to install them, so translated
captions always reported an unavailable model. Each manifest entry now names a commit-pinned
HTTPS `source` (validated: https, no query/fragment/backslash, files probe only). The installer
streams each missing file to `<file>.part`, hashes while writing, stops past the pinned size,
renames only on an exact match, then attests the directory. A same-size corrupt file is deleted
and fetched once more. Errors name files, never URLs.

`download_translation_models` installs only models that are neither attested internally nor
available from an external library, one at a time under `mutate`, so a model leased by live
captions is never touched and nothing loads a half-written install. Progress is one stream over
all models (about one event per MiB plus a final event per model).

Data: no migration. Pinning `revision` to the commit hash voids earlier `.nova-verified` markers;
such installs show as "Verify or repair" and re-attest without downloading. Cached translations
keyed by the old revision string are recomputed.

Non-goals: player-side download button, per-direction installs, resumable ranges, removal UI,
live/resident audio translation.

## Delivered and qualification

- Real network: all four models installed from Hugging Face through `install_pinned` (~443 MB,
  173 s on this connection); every file, including the small config/tokenizer files, matched its
  pin and was attested (`NOVA_TEST_INSTALL_OPUS_TO`).
- Real inference on those installs: en→fr, en→ar, fr→en ("It's beautiful today."), ar→en
  ("The weather is beautiful today.") and the fr→ar pivot through English all produce correct
  text; the runtime verifies once then translates (`NOVA_TEST_OPUS_DIR`). fr→en / ar→en were
  previously unmeasured.
- Unit tests: verified install + no second fetch, wrong content refused with no leftovers,
  oversize body stopped at the pin, same-size corruption repaired, URL never in errors, manifest
  source validation, Settings download/progress flow.
- Independent review: one medium finding (already-installed leased model blocked or unloaded by
  the command) and progress/labelling issues were fixed before commit.

Not qualified: the Settings button inside the native app, and translated captions end to end on
a provider stream after this install.
