#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUILD_DIR="$ROOT_DIR/build"

if [[ ! -d "$BUILD_DIR" ]]; then
  echo "Build folder not found: $BUILD_DIR"
  echo "Run tools/launcher/build-and-package.sh first."
  exit 1
fi

if [[ ! -f "$BUILD_DIR/miraset.exe" ]]; then
  echo "Missing $BUILD_DIR/miraset.exe"
  exit 1
fi

if [[ ! -f "$BUILD_DIR/miraset-worker.exe" ]]; then
  echo "Missing $BUILD_DIR/miraset-worker.exe"
  exit 1
fi

if [[ ! -f "$BUILD_DIR/wallet-miraset.exe" ]]; then
  echo "Missing $BUILD_DIR/wallet-miraset.exe"
  exit 1
fi

echo "Starting MIRASET node..."
"$BUILD_DIR/miraset.exe" node start &
NODE_PID=$!

sleep 2

echo "Starting MIRASET worker..."
"$BUILD_DIR/miraset-worker.exe" &
WORKER_PID=$!

sleep 1

echo "Starting MIRASET wallet GUI..."
"$BUILD_DIR/wallet-miraset.exe" &
WALLET_PID=$!

trap 'echo "Stopping..."; kill $NODE_PID $WORKER_PID $WALLET_PID 2>/dev/null || true' INT TERM

wait $NODE_PID $WORKER_PID $WALLET_PID

