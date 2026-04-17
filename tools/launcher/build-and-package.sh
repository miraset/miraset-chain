#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_DIR="$ROOT_DIR/build"
RELEASE_DIR="$ROOT_DIR/releases"

mkdir -p "$BUILD_DIR" "$RELEASE_DIR"

echo "Building node + worker + launcher (release)..."
cargo build --release --bin miraset --bin miraset-worker --bin miraset-launcher

echo "Building wallet (tauri)..."
(
  cd "$ROOT_DIR/wallet"
  bunx tauri build
)

echo "Copying artifacts..."
cp -f "$ROOT_DIR/target/release/miraset.exe" "$BUILD_DIR/miraset.exe"
cp -f "$ROOT_DIR/target/release/miraset-worker.exe" "$BUILD_DIR/miraset-worker.exe"
cp -f "$ROOT_DIR/target/release/miraset-launcher.exe" "$BUILD_DIR/miraset-launcher.exe"
cp -f "$ROOT_DIR/wallet/src-tauri/target/release/wallet-miraset.exe" "$BUILD_DIR/wallet-miraset.exe"

if [[ ! -f "$BUILD_DIR/miraset.exe" ]]; then
  echo "Missing $BUILD_DIR/miraset.exe"
  exit 1
fi
if [[ ! -f "$BUILD_DIR/miraset-worker.exe" ]]; then
  echo "Missing $BUILD_DIR/miraset-worker.exe"
  exit 1
fi
if [[ ! -f "$BUILD_DIR/miraset-launcher.exe" ]]; then
  echo "Missing $BUILD_DIR/miraset-launcher.exe"
  exit 1
fi
if [[ ! -f "$BUILD_DIR/wallet-miraset.exe" ]]; then
  echo "Missing $BUILD_DIR/wallet-miraset.exe"
  exit 1
fi

echo "Creating release archive..."
if command -v zip >/dev/null 2>&1; then
  (cd "$BUILD_DIR" && zip -r "$RELEASE_DIR/miraset-windows.zip" .)
else
  WIN_BUILD_DIR="$(cd "$BUILD_DIR" && pwd -W)"
  WIN_RELEASE_DIR="$(cd "$RELEASE_DIR" && pwd -W)"
  powershell.exe -NoProfile -Command "Compress-Archive -Path '$WIN_BUILD_DIR\\*' -DestinationPath '$WIN_RELEASE_DIR\\miraset-windows.zip' -Force"
fi

echo "Done: $RELEASE_DIR/miraset-windows.zip"
