#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
APP_NAME="OTel Desktop Viewer.app"
APP="${ROOT}/dist/native/${APP_NAME}"
CONTENTS="${APP}/Contents"
MACOS="${CONTENTS}/MacOS"
RESOURCES="${CONTENTS}/Resources"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script builds a macOS .app and must run on macOS." >&2
  exit 1
fi

cd "${ROOT}"
mkdir -p "${ROOT}/dist/native"
rm -rf "${APP}"
mkdir -p "${MACOS}" "${RESOURCES}"

# Go's cgo linker otherwise inherits this host's SDK deployment target, which
# can make the bundled helper incompatible with the app's macOS 15 minimum.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"

# Keep the helper at the canonical integration-smoke path as well as beside
# the app executable (the latter is how the packaged UI locates it).
EVALS=0 go build -trimpath -o "${ROOT}/dist/native/otel-desktop-backend" .
install -m 755 "${ROOT}/dist/native/otel-desktop-backend" "${MACOS}/otel-desktop-backend"
cargo build --locked --release --manifest-path native/Cargo.toml
install -m 755 native/target/release/otel-desktop-native "${MACOS}/otel-desktop-native"
mkdir -p "${RESOURCES}/Licenses"
cp native/assets/LICENSE-Madoromi "${RESOURCES}/Licenses/LICENSE-Madoromi"
cp native/assets/OFL-Atkinson.txt "${RESOURCES}/Licenses/OFL-Atkinson.txt"

cp LICENSE "${RESOURCES}/LICENSE"
cp native/packaging/Info.plist "${CONTENTS}/Info.plist"

ICONSET="${ROOT}/dist/native/AppIcon.iconset"
mkdir -p "${ICONSET}"
swift native/packaging/make-icon.swift "${ICONSET}/icon_1024.png"
for size in 16 32 64 128 256 512; do
  sips -z "${size}" "${size}" "${ICONSET}/icon_1024.png" --out "${ICONSET}/icon_${size}x${size}.png" >/dev/null
done
for size in 16 32 128 256 512; do
  double=$((size * 2))
  sips -z "${double}" "${double}" "${ICONSET}/icon_1024.png" --out "${ICONSET}/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "${ICONSET}" -o "${RESOURCES}/AppIcon.icns"
rm -rf "${ICONSET}"

# Ad-hoc signing makes the local app bundle launchable without a developer ID.
codesign --force --deep --sign - "${APP}"
codesign --verify --deep --strict "${APP}"
echo "Built ${APP}"
