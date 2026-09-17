# Nova stack and environment

`nova-stack.json` selects exact tool versions and a critical dependency inventory.
Cargo.lock and pnpm-lock.yaml remain the package-resolution authorities. This first
manifest is a development contract, not a signed native SDK/model manifest.

Commands (Node 22.22.3 + pnpm 9.15.4):

```sh
pnpm stack
pnpm doctor
pnpm verify-environment
pnpm verify-environment --manifest-only
pnpm test:environment
```

Equivalent `just stack`, `just doctor`, `just verify-environment` and
`just test-environment` recipes are available. JSON output: append `--json`.
Exit 0 means the selected check passed; 1 means drift/missing tool; 2 means invalid
input or unreadable repository files. `--manifest-only` intentionally skips runtime
tool probes for CI jobs without a full toolchain. Neither mode certifies native
libraries, model integrity, E2E, packaging or licensing; outstanding gaps are printed.

The commands are read-only, use shell-free probes with time/output bounds, never
load models or open profile databases, and do not echo subprocess stderr/environment.
The lock readers inspect only version fields in supported generated lock syntax;
they fail missing pins rather than guessing. Multiple transitive Rust versions are
allowed: an inventory entry asserts the named version is present, not exclusive.

## Setup

Install mise using its [official setup guide](https://mise.jdx.dev/getting-started.html),
then run `bash scripts/bootstrap.sh` (or `just setup` once just is available).
Bootstrap installs the exact versions in mise.toml, uses frozen/locked dependency
resolution and verifies the development tools. It does not download models, change
user data, install Homebrew media packages or claim the native app is ready to ship.
Those dependencies still need the versioned SDK workstream.

Rust 1.95.0, Node 22.22.3, pnpm 9.15.4 and just 1.51.0 were selected from the inspected
machine, without package upgrades. Rust was installed under the moving `stable`
alias. For this checkpoint only, tests used `RUSTUP_TOOLCHAIN=stable` after checking
that its actual rustc version is exactly 1.95.0; normal setup installs the named pin.
The default machine PATH selected Node 26.5.0 and omitted Cargo/just. Activate mise
or explicitly select installed tools; do not relax validation to make drift green.

No Python is introduced by these commands. Future Python evaluation/conversion
tooling should use uv, a local `.venv`, pyproject.toml and uv.lock, independently
of the native Rust/React application. No Docker wrapper is needed for macOS E2E.

## Changes and remaining work

Update manifest and relevant native lock/config files together in dedicated PRs.
Catalog fingerprint format v1 uses the existing locked sha2 0.10.9 dependency;
the core crate now declares that workspace dependency without upgrading any package.
The test-only `nova-iptv-fixture` workspace tool uses existing Axum/Tokio packages;
its parser clients are dev-dependencies, never dependencies of the desktop app.
CI validates repository consistency; `doctor` additionally compares executable
versions. The existing macOS CI still uses Homebrew mpv, so its native closure is
not reproducible yet. Model revision/hash verification, actual linked SQLite patch
inventory, Xcode/SDK pinning, clean-machine bootstrap, native bundling and E2E are
explicitly unresolved. See `docs/implementation/progress.md`.
