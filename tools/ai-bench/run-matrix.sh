#!/usr/bin/env bash
# Run the full AI22 candidate matrix and write one summary.
#
#   tools/ai-bench/run-matrix.sh /Volumes/Lexar/nova-ai/bench [asr|mt|all]
#
# Raw JSON reports stay under ROOT/results/<date>/ (outside the repo); the
# Markdown summary is printed at the end, to be copied to docs/evaluations/.
# Run on an idle machine, on power, after one warm-up run (Core ML compiles
# each encoder once; Apple engines fetch their assets on first use).
set -euo pipefail

ROOT=${1:?usage: run-matrix.sh ROOT [asr|mt|all]}
WHAT=${2:-all}
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$ROOT/results/$(date +%Y-%m-%d)"
mkdir -p "$OUT"

METAL="$HERE/target/release/nova-ai-bench"
COREML="$HERE/target-coreml/release/nova-ai-bench"
HELPER="$HERE/swift-helper/.build/release/nova-apple-helper"
W="$ROOT/models/whisper"
DATA="$ROOT/asr/fleurs"
LANGS=en_us,fr_fr,ar_eg

(cd "$HERE" && cargo build --release -q)
(cd "$HERE" && CARGO_TARGET_DIR=target-coreml cargo build --release -q --features coreml)
(cd "$HERE/swift-helper" && swift build -c release -q)

asr() { # name binary model step [extra args...]
  local name=$1 bin=$2 model=$3 step=$4; shift 4
  echo "== asr $name"
  "$bin" asr --name "$name" --model "$model" --data "$DATA" --langs "$LANGS" \
    --step "$step" --out "$OUT/asr-$name.json" "$@" 2>>"$OUT/log.txt"
}

if [[ $WHAT == asr || $WHAT == all ]]; then
  asr turbo-q5-coreml-s1.0   "$COREML" "$W/ggml-large-v3-turbo-q5_0.bin" 1.0
  asr turbo-q5-coreml-s0.5   "$COREML" "$W/ggml-large-v3-turbo-q5_0.bin" 0.5
  asr turbo-q8-coreml-s1.0   "$COREML" "$W/ggml-large-v3-turbo-q8_0.bin" 1.0
  asr turbo-q5-metal-s1.0    "$METAL"  "$W/ggml-large-v3-turbo-q5_0.bin" 1.0
  asr medium-q5-coreml-s1.0  "$COREML" "$W/ggml-medium-q5_0.bin" 1.0
  asr medium-q5-coreml-s0.5  "$COREML" "$W/ggml-medium-q5_0.bin" 0.5
  asr small-q5-coreml-s1.0   "$COREML" "$W/ggml-small-q5_1.bin" 1.0
  asr small-q5-coreml-s0.5   "$COREML" "$W/ggml-small-q5_1.bin" 0.5
  for mode in standard fast; do
    flag=(); [[ $mode == fast ]] && flag=(--fast)
    echo "== asr apple-$mode"
    "$METAL" asr --engine apple "${flag[@]}" --name "apple-$mode" --helper "$HELPER" \
      --data "$DATA" --langs en_us,fr_fr --out "$OUT/asr-apple-$mode.json" 2>>"$OUT/log.txt"
  done
fi

if [[ $WHAT == mt || $WHAT == all ]]; then
  PAIRS=en-fr,fr-en,en-ar,ar-en,fr-ar,ar-fr
  FLORES="$ROOT/mt/flores200/devtest"
  echo "== mt opus";  "$METAL" mt --name opus-mt --engine opus --models "$ROOT/models/opus" --pairs $PAIRS --flores "$FLORES" --out "$OUT/mt-opus.json" 2>>"$OUT/log.txt"
  echo "== mt nllb";  "$METAL" mt --name nllb-600m --engine nllb --models "$ROOT/models/nllb" --pairs $PAIRS --flores "$FLORES" --out "$OUT/mt-nllb.json" 2>>"$OUT/log.txt"
  echo "== mt translategemma"; "$METAL" mt --name translategemma-4b-q4 --engine llama --models "$ROOT/models/llm/translategemma-4b-it.Q4_K_M.gguf" --pairs $PAIRS --flores "$FLORES" --out "$OUT/mt-translategemma.json" 2>>"$OUT/log.txt"
  echo "== mt apple"; "$METAL" mt --name apple-translation --engine apple --models "$HELPER" --pairs $PAIRS --flores "$FLORES" --out "$OUT/mt-apple.json" 2>>"$OUT/log.txt" \
    || echo "   apple translation skipped (language pairs not installed?)"
fi

"$METAL" report "$OUT"/*.json --out "$OUT/summary.md"
echo "summary: $OUT/summary.md"
