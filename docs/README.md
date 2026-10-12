# Nova documentation

## Using Nova

- [Getting started](guides/getting-started.md): install, first playlist, models, troubleshooting
- [On-device intelligence](features/intelligence.md): captions, translation, search, model management, failure behaviour

## Understanding Nova

- [Architecture overview](architecture/overview.md): processes, crates, data ownership, playback, the model runtime
- [Translation pipeline](translation-pipeline.md): how captions become translated captions
- [Catalog contracts](catalog.md): sources, snapshots, fingerprints and canonical identity
- [Long-term architecture](architecture/long-term-architecture.md): the target design and milestone process
- [Architecture decision records](adr/)

## Contributing to Nova

- [Contributing guide](../CONTRIBUTING.md): workflow, conventions and checks
- [IPC contract](ipc-contract.md): every Tauri command and event
- [Testing strategy](testing.md)
- [Design tokens](design-tokens.md) and the [player motion contract](player-motion-frontend-contract.md)
- [Stack and environment](../STACK.md)

## Project records

- [Implementation status](nova-implementation-status.md): verified milestones, the reconciliation matrix and open gates
- [Implementation contracts](implementation/): per-milestone contracts and validation records
- [Evaluations](evaluations/): model and benchmark reports
- [Changelog](../CHANGELOG.md)
- [Archive](archive/README.md): superseded plans, kept for history only
