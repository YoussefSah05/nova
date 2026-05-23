#!/usr/bin/env bash
# Downloads and converts Whisper models to Core ML format for Apple Silicon.
# Implemented in Phase 6. For now this is a Phase 0 stub.
set -euo pipefail

echo "[nova/convert-whisper] Phase 0 stub — Whisper model conversion deferred to Phase 6."
echo "[nova/convert-whisper] In production, this script downloads large-v3-turbo (q5_0 ~500MB)"
echo "[nova/convert-whisper] from Hugging Face, converts to Core ML via whisper.cpp tooling,"
echo "[nova/convert-whisper] and caches .mlmodelc at ~/Library/Application Support/Nova/models/whisper/."
