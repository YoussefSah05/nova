# Nova documentation

## Using Nova

- [Getting started](guides/getting-started.md): install, add a playlist, download models, troubleshooting
- [User guide](guides/user-guide.md): every screen, the player, captions, settings and keyboard shortcuts
- [On-device intelligence](features/intelligence.md): captions, translation, search, model management, failure behaviour
- [FAQ](FAQ.md)

## Understanding Nova

- [Architecture overview](architecture/overview.md): processes, crates, data ownership, playback, the model runtime
- [Performance engineering](architecture/performance.md): budgets, measurement tools, case studies, rules of thumb
- [Translation pipeline](translation-pipeline.md): how captions become translated captions
- [Catalog contracts](catalog.md): sources, snapshots, fingerprints and canonical identity
- [Long-term architecture](architecture/long-term-architecture.md): the target design and milestone process
- [Architecture decision records](adr/), including
  [why the interface is web-based](adr/0013-web-ui-on-tauri-with-native-video.md)

## Contributing to Nova

- [Contributing guide](../CONTRIBUTING.md) and [code of conduct](../CODE_OF_CONDUCT.md)
- [IPC contract](ipc-contract.md): every Tauri command and event
- [Testing strategy](testing.md), including measuring scroll smoothness
- [Design tokens](design-tokens.md) and the [player motion contract](player-motion-frontend-contract.md)
- Reference notes: [FFmpeg](reference/ffmpeg.md), [stream proxy](reference/hls-proxy.md),
  [Whisper](reference/whisper.md), [XMLTV guides](reference/xmltv-epg.md),
  [validating imports](reference/import-validation.md)
- [Stack and environment](../STACK.md)

## Project records

- [Roadmap](roadmap.md)
- [Implementation status](nova-implementation-status.md): verified milestones, the reconciliation matrix and open gates
- [Implementation contracts](implementation/): per-milestone contracts and validation records
- [Evaluations](evaluations/): model and benchmark reports
- [Brand](brand.md): the logo, its story, colours and usage
- [Changelog](../CHANGELOG.md)
- [Security policy](../SECURITY.md)
- [Archive](archive/README.md): superseded plans, kept for history only
