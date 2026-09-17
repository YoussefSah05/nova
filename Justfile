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
