#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist/MIRASET_WALLET"
ZIP_PATH="$ROOT_DIR/dist/miraset_wallet.zip"

MIRASET_BIN="$ROOT_DIR/target/release/miraset.exe"
WALLET_BIN="$ROOT_DIR/wallet/src-tauri/target/release/wallet-miraset.exe"

mkdir -p "$DIST_DIR"

if [[ ! -f "$MIRASET_BIN" ]]; then
  echo "Missing $MIRASET_BIN. Build it with: cargo build --release --bin miraset" >&2
  exit 1
fi

if [[ ! -f "$WALLET_BIN" ]]; then
  echo "Missing $WALLET_BIN. Build it with: cd wallet && bun run tauri:build" >&2
  exit 1
fi

cp "$MIRASET_BIN" "$DIST_DIR/miraset.exe"
cp "$WALLET_BIN" "$DIST_DIR/wallet-miraset.exe"

mkdir -p "$ROOT_DIR/dist"

powershell.exe -NoProfile -Command "Compress-Archive -Force -Path '$DIST_DIR/*' -DestinationPath '$ZIP_PATH'"

echo "ZIP created: $ZIP_PATH"

