#!/usr/bin/env bash
# Explicit developer setup/repair. Never downloads model weights or edits PATH.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=1.8.3
SHA256=870ba21409cdf66697dc4db15ebdb13bc67037d76c7cc63756c81471d8f1731a
URL="https://codeload.github.com/ggml-org/whisper.cpp/tar.gz/refs/tags/v${VERSION}"
if (( $# > 1 )); then
  echo 'Usage: bash scripts/install-whisper-cli.sh [absolute-runtime-directory]' >&2
  exit 1
fi
for tool in cmake curl tar node; do
  command -v "$tool" >/dev/null || { echo "Required build tool missing: ${tool}" >&2; exit 1; }
done
APP_ID="$(node -e 'console.log(JSON.parse(require("fs").readFileSync("apps/desktop/src-tauri/tauri.conf.json", "utf8")).identifier)')"
case "$(uname -s)" in
  Darwin)
    DEFAULT_RUNTIME="${HOME}/Library/Application Support/${APP_ID}/runtime/whisper"
    ACCEL=(-DWHISPER_COREML=ON -DWHISPER_COREML_ALLOW_FALLBACK=ON -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0)
    ;;
  Linux)
    DEFAULT_RUNTIME="${XDG_DATA_HOME:-${HOME}/.local/share}/${APP_ID}/runtime/whisper"
    ACCEL=(-DWHISPER_COREML=OFF -DGGML_METAL=OFF)
    ;;
  *) echo 'This installer supports macOS and Linux.' >&2; exit 1 ;;
esac
RUNTIME="${1:-${DEFAULT_RUNTIME}}"
[[ "$RUNTIME" == /* ]] || { echo 'Runtime directory must be absolute.' >&2; exit 1; }

mkdir -p target/whisper-cli
ARCHIVE="${PWD}/target/whisper-cli/whisper-${VERSION}.tar.gz"
verify_archive() {
  if command -v shasum >/dev/null; then
    printf '%s  %s\n' "$SHA256" "$ARCHIVE" | shasum -a 256 -c - >/dev/null
  else
    printf '%s  %s\n' "$SHA256" "$ARCHIVE" | sha256sum -c - >/dev/null
  fi
}
if [[ ! -f "$ARCHIVE" ]]; then
  curl --fail --location --proto '=https' --tlsv1.2 --retry 2 --max-time 180 --output "${ARCHIVE}.download" "$URL"
  mv "${ARCHIVE}.download" "$ARCHIVE"
fi
verify_archive || { echo 'Whisper source checksum mismatch; no runtime changes made.' >&2; exit 1; }

WORK="$(mktemp -d "${PWD}/target/whisper-cli/build.XXXXXX")"
STAGED=''
cleanup() {
  [[ -z "$STAGED" ]] || rm -f "$STAGED"
  rm -rf "$WORK"
}
trap cleanup EXIT
tar -xzf "$ARCHIVE" -C "$WORK"
echo "Building whisper.cpp ${VERSION} with Nova's acceleration settings…"
cmake -S "$WORK/whisper.cpp-${VERSION}" -B "$WORK/build" \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF -DGGML_BACKEND_DL=OFF \
  -DGGML_OPENMP=OFF -DGGML_NATIVE=OFF -DWHISPER_BUILD_EXAMPLES=ON \
  -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF "${ACCEL[@]}"
cmake --build "$WORK/build" --target whisper-cli --config Release --parallel "${CMAKE_BUILD_PARALLEL_LEVEL:-4}"
BIN="$WORK/build/bin/whisper-cli"
"$BIN" --help > "$WORK/help.txt" 2>&1
for flag in --output-json-full --dtw --max-len --split-on-word --prompt; do
  grep -F -- "$flag" "$WORK/help.txt" >/dev/null || { echo "Required Whisper feature missing: ${flag}" >&2; exit 1; }
done

# Publish an already-built executable atomically. Existing inference can finish
# with the old inode; future requests see the replacement. No global uninstall.
mkdir -p "$RUNTIME"
STAGED="$(mktemp "${RUNTIME}/.whisper-cli.XXXXXX")"
cp "$BIN" "$STAGED"
chmod 755 "$STAGED"
mv -f "$STAGED" "${RUNTIME}/whisper-cli"
STAGED=''
printf 'whisper.cpp %s\nsource-sha256 %s\n' "$VERSION" "$SHA256" > "${RUNTIME}/build-info.txt"
echo "Nova Whisper runtime installed (${VERSION}). Model weights were preserved."
