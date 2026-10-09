#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v mise >/dev/null 2>&1; then
  echo 'mise is required. Install it using https://mise.jdx.dev/getting-started.html, then rerun bash scripts/bootstrap.sh.' >&2
  exit 1
fi

# Explicit developer setup only; doctor/stack never install anything.
mise install
mise exec -- pnpm install --frozen-lockfile
mise exec -- cargo fetch --locked
mise exec -- bash scripts/install-whisper-cli.sh
mise exec -- pnpm verify-environment
echo 'Toolchain and package setup complete. Native SDK/model readiness remains separate; see STACK.md.'
