#!/usr/bin/env bash
# Simulates running the Miraset node + block resource benchmark on each
# Namecheap VPS package by running them inside systemd cgroup scopes with
# CPUQuota/MemoryMax matching the package specs.
#
# Package specs/prices: https://www.namecheap.com/hosting/vps/ (2026).
#
# Usage: bash scripts/bench-vps-sim.sh [package...]
#   (no args = all packages; names: Spark Pulsar Quasar Magnetar Hypernova)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# name  cpu  ram_mb  storage_gb  price_usd_mo
PACKAGES=(
  "Spark 1 1024 20 3.88"
  "Pulsar 2 2048 40 6.88"
  "Quasar 4 6144 120 12.88"
  "Magnetar 8 12288 240 24.88"
  "Hypernova 12 24576 500 46.88"
)

if [[ $# -gt 0 ]]; then
  selected=()
  for pkg in "${PACKAGES[@]}"; do
    name="${pkg%% *}"
    for want in "$@"; do
      if [[ "${name,,}" == "${want,,}" ]]; then
        selected+=("$pkg")
      fi
    done
  done
  if [[ ${#selected[@]} -eq 0 ]]; then
    echo "no packages matched: $*" >&2
    exit 2
  fi
  PACKAGES=("${selected[@]}")
fi

echo "[vps-sim] building node + bench binary ..."
cargo build --release --bin miraset --quiet
cargo test -p miraset-node --test block_resource_bench --release --no-run --quiet
BENCH_BIN="$(ls -t "$ROOT"/target/release/deps/block_resource_bench-* 2>/dev/null | while read -r f; do
  [[ -x "$f" && "$f" != *.d ]] && echo "$f" && break
done)"
if [[ -z "$BENCH_BIN" ]]; then
  echo "bench binary not found in target/release/deps" >&2
  exit 1
fi
echo "[vps-sim] bench binary: $BENCH_BIN"

mkdir -p "$ROOT/reports/vps-sim"
failed=()
for pkg in "${PACKAGES[@]}"; do
  read -r name cpu ram storage price <<<"$pkg"
  echo
  echo "=== $name: ${cpu} CPU, ${ram} MB RAM, ${storage} GB, \$${price}/mo ==="
  if ! systemd-run --user --scope --quiet \
    -p "CPUQuota=$((cpu * 100))%" \
    -p "MemoryMax=${ram}M" \
    -p "MemorySwapMax=0" \
    bash "$ROOT/scripts/vps-sim-run.sh" "$name" "$cpu" "$ram" "$storage" "$price" "$BENCH_BIN"; then
    echo "[$name] FAILED" >&2
    failed+=("$name")
  fi
done

echo
if [[ ${#failed[@]} -gt 0 ]]; then
  echo "[vps-sim] failed packages: ${failed[*]}" >&2
  exit 1
fi

echo "[vps-sim] all packages done; generating aggregated report ..."
python3 "$ROOT/scripts/vps-sim-report.py"
