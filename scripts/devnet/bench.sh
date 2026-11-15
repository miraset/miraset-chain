#!/usr/bin/env bash
# Measured throughput / finality sweep against the 5-validator docker devnet (TODO 8.5.3).
#
# For each offered rate: run `miraset-chain loadgen` inside the devnet network (transfers plus
# compute receipts, every sender pinned to one node), save its JSON report, then check that all
# validators still agree at every height. The environment (hardware, kernel, Docker, git
# revision, netem profile) is recorded next to the reports. Results are a devnet baseline only:
# no throughput or latency target is approved (PRD).
#
# Usage (repo root, with `docker compose up -d --build` running):
#   scripts/devnet/bench.sh [--profile lan|wan|wan-far] [--rates "10 20 40"] [--duration 120]
#                           [--warmup 20] [--senders 32] [--receipt-share 0.15] [--out DIR]
#                           [--check-heights N]
# --check-heights N checks agreement on the newest N heights after each rate instead of every
# height (0, the default). With empty blocks at tens per second per validator, a full walk grows
# by ~100k heights per half hour; a window larger than the heights committed since the previous
# check keeps every height checked once.
set -euo pipefail

COMPOSE="${COMPOSE:-docker compose}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALL_RPCS="http://127.0.0.1:8545,http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548,http://127.0.0.1:8549"
IN_NETWORK_RPCS=(--rpc http://node0:8545 --rpc http://node1:8545 --rpc http://node2:8545 --rpc http://node3:8545 --rpc http://node4:8545)
# Per-item cost (amount 1 + fee 10, or receipt cost 1), doubled for headroom.
ITEM_BUDGET=22

profile=lan rates="10 20 40" duration=120 warmup=20 senders=32 share=0.15 out="" check_heights=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile) profile="$2"; shift 2 ;;
    --rates) rates="$2"; shift 2 ;;
    --duration) duration="$2"; shift 2 ;;
    --warmup) warmup="$2"; shift 2 ;;
    --senders) senders="$2"; shift 2 ;;
    --receipt-share) share="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --check-heights) check_heights="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 64 ;;
  esac
done
out="${out:-bench-results/$(date -u +%Y%m%dT%H%M%SZ)-$profile}"
mkdir -p "$out"

# Agreement is checked at every height with shaping cleared: under netem each of the
# height x node requests would carry the WAN delay and the check would take hours.
check_agreement() {
  [[ "$profile" == lan ]] || "$HERE/netem.sh" lan >/dev/null
  python3 "$HERE/check_consensus.py" --rpc "$ALL_RPCS" --min-reachable 5 \
    --max-heights "$check_heights"
  [[ "$profile" == lan ]] || "$HERE/netem.sh" "$profile" >/dev/null
}

fund_amount() { # $1 = rate
  python3 -c "import math; print(math.ceil($1 * $duration / $senders) * $ITEM_BUDGET + 1000)"
}

record_environment() {
  {
    echo "recorded_utc: $(date -u +%FT%TZ)"
    echo "git: $(git rev-parse HEAD)$([[ -z "$(git status --porcelain)" ]] || echo ' (uncommitted changes)')"
    echo "image: $(docker image inspect miraset-chain:devnet --format '{{.Id}}')"
    echo "cpu: $(lscpu | sed -n 's/^Model name: *//p')"
    echo "cpus: $(nproc)"
    echo "memory: $(free -h | awk '/^Mem:/ {print $2}')"
    echo "kernel: $(uname -r)"
    echo "docker: $(docker version --format '{{.Server.Version}}')"
    echo "profile: $profile"
    echo "netem:"; "$HERE/netem.sh" show | sed 's/^/  /'
    echo "validators: 5 (equal power, f = 1), max view time 2s (default), empty-block heartbeat 1s"
    echo "shaping: validator egress only; loadgen requests unshaped, responses and commit events shaped"
    echo "load: senders=$senders receipt_share=$share duration=${duration}s warmup=${warmup}s rates=[$rates]"
  } >"$out/environment.txt"
}

trap '"$HERE/netem.sh" lan >/dev/null || true' EXIT
python3 "$HERE/check_consensus.py" --rpc "$ALL_RPCS" --min-reachable 5
"$HERE/netem.sh" "$profile"
record_environment

for rate in $rates; do
  echo "=== offered rate $rate/s ($profile) ==="
  $COMPOSE run --rm -T loadgen loadgen "${IN_NETWORK_RPCS[@]}" \
    --faucet-key /keys/faucet/faucet.key --coordinator-key /keys/coordinator/coordinator.key \
    --senders "$senders" --rate "$rate" \
    --receipt-share "$share" --duration-secs "$duration" --warmup-secs "$warmup" \
    --fund-amount "$(fund_amount "$rate")" >"$out/rate-$rate.json" 2>"$out/rate-$rate.log"
  check_agreement
done

python3 "$HERE/bench_summary.py" "$out" | tee "$out/summary.md"
echo "results: $out"
