#!/usr/bin/env bash
# Runs INSIDE the systemd scope that emulates one Namecheap VPS package
# (CPUQuota + MemoryMax applied by scripts/bench-vps-sim.sh).
#
# Starts a Miraset node, runs the block resource benchmark under the same
# cgroup limits, samples node/CPU/RAM metrics, and writes per-package
# JSON files to reports/vps-sim/.
#
# Usage: vps-sim-run.sh <name> <cpu> <ram_mb> <storage_gb> <price_usd> <bench_bin>
set -euo pipefail

if [[ $# -ne 6 ]]; then
  echo "usage: $0 <name> <cpu> <ram_mb> <storage_gb> <price_usd> <bench_bin>" >&2
  exit 2
fi

PKG_NAME="$1"
PKG_CPU="$2"
PKG_RAM_MB="$3"
PKG_STORAGE_GB="$4"
PKG_PRICE="$5"
BENCH_BIN="$6"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$ROOT/reports/vps-sim"
STORE="$(mktemp -d /tmp/miraset-vps-sim.XXXXXX)"
NODE_LOG="$STORE/node.log"
RSS_FILE="$STORE/rss-samples.txt"
mkdir -p "$OUT_DIR"
: >"$RSS_FILE"

NODE_PID=""
SAMPLER_PID=""

cleanup() {
  if [[ -n "$SAMPLER_PID" ]]; then
    kill "$SAMPLER_PID" 2>/dev/null || true
    wait "$SAMPLER_PID" 2>/dev/null || true
  fi
  if [[ -n "$NODE_PID" ]]; then
    kill "$NODE_PID" 2>/dev/null || true
    wait "$NODE_PID" 2>/dev/null || true
  fi
  rm -rf "$STORE"
}
trap cleanup EXIT

# --- cgroup of this scope (whole simulated VPS) ---------------------------
cgroup_path="$(awk -F: '{print $3}' /proc/self/cgroup)"
cgroup_dir="/sys/fs/cgroup${cgroup_path}"

read_cgroup_cpu_us() {
  awk '/^usage_usec/{print $2}' "$cgroup_dir/cpu.stat" 2>/dev/null || echo ""
}

# --- start node ------------------------------------------------------------
MIRASET_STORAGE_PATH="$STORE" \
MIRASET_BLOCK_INTERVAL=5 \
MIRASET_RPC_ADDR=127.0.0.1:9944 \
  "$ROOT/target/release/miraset" node start >"$NODE_LOG" 2>&1 &
NODE_PID=$!

healthy=0
for _ in $(seq 1 60); do
  if curl -sf --max-time 1 http://127.0.0.1:9944/health >/dev/null 2>&1; then
    healthy=1
    break
  fi
  sleep 0.5
done
if [[ "$healthy" != "1" ]]; then
  echo "[$PKG_NAME] node failed to start" >&2
  cat "$NODE_LOG" >&2
  exit 1
fi

height_start="$(curl -sf --max-time 2 http://127.0.0.1:9944/health | python3 -c 'import json,sys; print(json.load(sys.stdin)["latest_block_height"])')"
# node CPU time (utime+stime, clk ticks = 100) -> ms
node_cpu0_ms="$(awk '{printf "%.0f", ($14+$15)*10}' "/proc/$NODE_PID/stat")"

# --- RSS sampler -------------------------------------------------------------
(
  while :; do
    rss="$(awk '/^VmRSS:/{print $2}' "/proc/$NODE_PID/status" 2>/dev/null || true)"
    if [[ -n "$rss" ]]; then
      echo "$rss" >>"$RSS_FILE"
    fi
    sleep 0.2
  done
) &
SAMPLER_PID=$!

# --- run benchmark under package limits ------------------------------------
cgroup_cpu0_us="$(read_cgroup_cpu_us)"
bench_start="$(date +%s.%N)"
set +e
MIRASET_BENCH_PACKAGE="$PKG_NAME" \
MIRASET_BENCH_PACKAGE_CPU="$PKG_CPU" \
MIRASET_BENCH_PACKAGE_RAM_MB="$PKG_RAM_MB" \
MIRASET_BENCH_PACKAGE_STORAGE_GB="$PKG_STORAGE_GB" \
MIRASET_BENCH_PACKAGE_PRICE="$PKG_PRICE" \
MIRASET_BENCH_OUT="$OUT_DIR/$PKG_NAME" \
  "$BENCH_BIN" --ignored --nocapture block_resource_benchmark
bench_rc=$?
set -e
bench_end="$(date +%s.%N)"
cgroup_cpu1_us="$(read_cgroup_cpu_us)"

# --- collect metrics ---------------------------------------------------------
height_end="$(curl -sf --max-time 2 http://127.0.0.1:9944/health | python3 -c 'import json,sys; print(json.load(sys.stdin)["latest_block_height"])' || echo "$height_start")"
node_cpu1_ms="$(awk '{printf "%.0f", ($14+$15)*10}' "/proc/$NODE_PID/stat" 2>/dev/null || echo "$node_cpu0_ms")"
node_peak_rss_kb="$(sort -n "$RSS_FILE" | tail -1 || echo "")"
[[ -n "$node_peak_rss_kb" ]] || node_peak_rss_kb=0

cgroup_peak_ram_kb="$(tr -dc '0-9' <"$cgroup_dir/memory.peak" 2>/dev/null || true)"
if [[ -n "$cgroup_peak_ram_kb" ]]; then
  cgroup_peak_ram_kb=$((cgroup_peak_ram_kb / 1024))
fi

bench_wall_ms="$(python3 -c "print(int((float('$bench_end')-float('$bench_start'))*1000))")"
cgroup_cpu_ms=""
if [[ -n "$cgroup_cpu0_us" && -n "$cgroup_cpu1_us" ]]; then
  cgroup_cpu_ms="$(((cgroup_cpu1_us - cgroup_cpu0_us) / 1000))"
fi
node_wall_ms="$bench_wall_ms"
node_cpu_ms=$((node_cpu1_ms - node_cpu0_ms))
node_cpu_pct="0"
if [[ "$node_wall_ms" -gt 0 ]]; then
  node_cpu_pct="$(python3 -c "print(round($node_cpu_ms / $node_wall_ms * 100, 1))")"
fi

# stop node, then measure storage footprint
kill "$NODE_PID" 2>/dev/null || true
wait "$NODE_PID" 2>/dev/null || true
NODE_PID=""
storage_bytes="$(du -sb "$STORE" 2>/dev/null | cut -f1 || echo 0)"

# --- write node metrics JSON ---------------------------------------------------
python3 - "$OUT_DIR/$PKG_NAME.node.json" <<PY
import json, sys
json.dump({
    "package": "$PKG_NAME",
    "cpu_cores": int("$PKG_CPU"),
    "ram_mb": int("$PKG_RAM_MB"),
    "storage_gb": int("$PKG_STORAGE_GB"),
    "price_usd_mo": float("$PKG_PRICE"),
    "node_started": True,
    "bench_exit_code": int("$bench_rc"),
    "height_start": int("$height_start"),
    "height_end": int("$height_end"),
    "blocks_produced": int("$height_end") - int("$height_start"),
    "node_peak_rss_kb": int("$node_peak_rss_kb"),
    "node_cpu_ms": int("$node_cpu_ms"),
    "node_cpu_percent": float("$node_cpu_pct"),
    "bench_wall_ms": int("$bench_wall_ms"),
    "cgroup_peak_ram_kb": int("${cgroup_peak_ram_kb:-0}"),
    "cgroup_cpu_ms": int("${cgroup_cpu_ms:-0}"),
    "node_storage_bytes": int("$storage_bytes"),
}, open(sys.argv[1], "w"), indent=2)
PY

echo "[$PKG_NAME] done: bench_rc=$bench_rc node_peak_rss=${node_peak_rss_kb}KiB cgroup_peak=${cgroup_peak_ram_kb:-?}KiB blocks=$height_start->$height_end"

exit "$bench_rc"
