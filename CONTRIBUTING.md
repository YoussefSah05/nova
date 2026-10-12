# Contributing to Nova

Thanks for your interest in Nova! This guide covers how to set up, how work is organised, and
what a change needs before it can merge.

## Setup

Follow [Getting started](docs/guides/getting-started.md), then confirm your toolchain:

```bash
just doctor   # compares installed tools with the pinned stack
just test     # Rust + TypeScript tests
```

## How work is organised

- Architecture follows [the master bootstrap](docs/architecture/long-term-architecture.md).
  Check [the implementation status](docs/nova-implementation-status.md) before starting: extend
  existing modules instead of building parallel replacements.
- One milestone or fix per branch off `main` (`feat/...`, `fix/...`, `docs/...`).
- Before editing, write a short implementation contract: what you reuse, the missing behaviour,
  data/migration impact, concurrency and resource impact, UX impact, tests, non-goals. Examples
  live in [`docs/implementation/`](docs/implementation/).
- Record significant decisions as an [ADR](docs/adr/).
- Finish by updating `docs/nova-implementation-status.md` with what was validated and what was not.

## Before you open a pull request

```bash
just format
just lint && just typecheck && just test
```

CI runs the same checks (Clippy with `-D warnings`, rustfmt, ESLint, `tsc`, Vitest) and a failing
check blocks the merge. The [pull-request template](.github/PULL_REQUEST_TEMPLATE.md) lists what
reviewers look for.

For anything visible or performance-related, also check a release build (`just build`). Measure
scrolling with the frame-rate overlay (⌘⌥F) and queries with the `live_perf` probe, and include
before/after numbers; see [Performance engineering](docs/architecture/performance.md).

## Conventions

### Rust

- Workspace dependencies only: declare versions in the root `Cargo.toml` and use
  `dep = { workspace = true }` in crates.
- Errors: `thiserror` in libraries, `anyhow` only in binaries and tests. No `String` errors.
- SQL: compile-time checked `sqlx::query!` / `query_as!` only. After changing queries, refresh
  the offline metadata in `.sqlx/` and commit it with the change.
- Migrations are additive and numbered; never edit an applied migration.
- All I/O is async Tokio; never block the executor (use `spawn_blocking` for CPU or native work).
- Model work goes through the model runtime and resource governor, never around them.

### TypeScript and React

- Strict mode, no `any`. IPC payload types are generated from Rust into
  `apps/desktop/src/types/generated/`; never hand-write them. Regenerate with `just bindings`.
- Use `@/` imports and `cn()` for conditional classes; follow the
  [design tokens](docs/design-tokens.md).

### IPC

- Commands return `Result<T, ApiError>`; events use colon-separated names
  (`playlist:import:progress`). Document additions in [the IPC contract](docs/ipc-contract.md).
- Never send provider URLs, credentials or file paths to the webview.

### Commits

[Conventional Commits](https://www.conventionalcommits.org/): `feat(m18): ...`,
`fix(transcribe): ...`, `docs: ...`, `chore: ...`. Explain the why in the body.

## Reporting bugs

Open an issue with steps to reproduce, what you expected, and `just doctor` output. Never paste
playlist URLs, provider credentials or TMDB keys. Report security issues privately (see
[SECURITY.md](SECURITY.md)).
