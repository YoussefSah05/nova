# mise shims must be on PATH so cargo/rustfmt/clippy are found in every recipe,
# including immediately after `mise install` runs in the same `just setup` call.
export PATH := env_var("HOME") + "/.local/share/mise/shims:" + env_var("PATH")
export SQLX_OFFLINE := "true"

default:
    @just --list

# Install all toolchain dependencies and fetch packages
setup:
    bash scripts/bootstrap.sh

# Read-only stack inventory and drift checks (no model downloads)
stack:
    pnpm stack

doctor:
    pnpm doctor

verify-environment:
    pnpm verify-environment

test-environment:
    pnpm test:environment

# Synthetic loopback provider (Ctrl+C stops it); no playable media yet
fixture-provider:
    cargo run -p nova-iptv-fixture --locked

test-fixtures:
    cargo test -p nova-iptv-fixture --locked

# Synthetic data root for native profile-recovery validation (absolute, empty dir)
recovery-fixture dir:
    cargo run -p nova-storage --locked --example recovery_validation_fixture -- {{dir}}

# Debug app with its own identity, pointed at a synthetic data root (never real profiles)
# A relative path is ignored by the app, which would then open the real data root.
dev-validation dir:
    @case "{{dir}}" in /*) ;; *) echo "dir must be an absolute path"; exit 1;; esac
    @test -f "{{dir}}/app.db" || { echo "no synthetic app.db; run: just recovery-fixture {{dir}}"; exit 1; }
    NOVA_VALIDATION_DIR={{dir}} pnpm -C apps/desktop exec tauri dev --config src-tauri/tauri.validation.conf.json

# Start the Tauri dev window
dev:
    pnpm dev

# Build a production Tauri bundle
build:
    pnpm build

# Run all tests (Rust + TypeScript)
test:
    cargo test --workspace --locked
    pnpm test

# Lint everything (Rust + TypeScript)
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings
    pnpm lint

# Auto-format everything (Rust + TypeScript)
format:
    cargo fmt --all
    pnpm format

# TypeScript type-check
typecheck:
    pnpm typecheck

# Regenerate Rust-owned IPC bindings and format the generated TypeScript
bindings:
    cargo test -p nova-storage --lib export_bindings --locked
    cargo test -p nova-desktop --lib export_bindings --locked
    pnpm -C apps/desktop exec prettier --write src/types/generated

# Clean all build artifacts
clean:
    cargo clean
    pnpm clean

# Check Rust without producing artifacts (fast)
check:
    cargo check --workspace --locked

# Run only Rust tests
test-rust:
    cargo test --workspace --locked

# Run only TypeScript tests
test-ts:
    pnpm test
