#!/usr/bin/env bash
# Stability soak against the 5-validator docker devnet (TODO 8.5.3).
#
# Runs `loadgen` at a constant offered rate for the whole duration while sampling, per
# validator: resident memory, open file descriptors, chain data size, committed height and
# container restarts. At the end it checks agreement at every height and greps for safety
# halts, then summarises growth (scripts/devnet/soak_summary.py). The 72-hour run is
# `--duration 72h`; it records evidence, it does not judge it against an unapproved target.
#
# Usage (repo root, with `docker compose up -d --build` running):
#   scripts/devnet/soak.sh [--duration 30m|72h|<secs>] [--rate 20] [--interval 60]
#                          [--profile lan|wan|wan-far] [--senders 32] [--out DIR]
set -euo pipefail

COMPOSE="${COMPOSE:-docker compose}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALL_RPCS="http://127.0.0.1:8545,http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548,http://127.0.0.1:8549"
IN_NETWORK_RPCS=(--rpc http://node0:8545 --rpc http://node1:8545 --rpc http://node2:8545 --rpc http://node3:8545 --rpc http://node4:8545)
ITEM_BUDGET=22

duration=30m rate=20 interval=60 profile=lan senders=32 out=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --duration) duration="$2"; shift 2 ;;
    --rate) rate="$2"; shift 2 ;;
    --interval) interval="$2"; shift 2 ;;
    --profile) profile="$2"; shift 2 ;;
    --senders) senders="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 64 ;;
  esac
done

to_secs() {
  case "$1" in
    *h) echo $(( ${1%h} * 3600 )) ;;
    *m) echo $(( ${1%m} * 60 )) ;;
    *s) echo "${1%s}" ;;
    *) echo "$1" ;;
  esac
}
secs="$(to_secs "$duration")"
warmup=$(( secs / 10 < 300 ? secs / 10 : 300 ))
out="${out:-bench-results/$(date -u +%Y%m%dT%H%M%SZ)-soak-$duration-$profile}"
mkdir -p "$out"
fund="$(python3 -c "import math; print(math.ceil($rate * $secs / $senders) * $ITEM_BUDGET + 1000)")"

sample() { # one CSV row per validator
  local ts node cid rss fds data height restarts
  ts="$(date -u +%s)"
  for i in 0 1 2 3 4; do
    node="node$i"
    cid="$($COMPOSE ps -q "$node" 2>/dev/null || true)"
    rss="$(docker exec "$cid" sh -c "awk '/^VmRSS/ {print \$2 * 1024}' /proc/1/status" 2>/dev/null || echo)"
    fds="$(docker exec "$cid" sh -c 'ls /proc/1/fd | wc -l' 2>/dev/null || echo)"
    data="$(docker exec "$cid" du -sb /var/lib/miraset 2>/dev/null | cut -f1 || echo)"
    height="$(curl -fsS --max-time 2 "http://127.0.0.1:$((8545 + i))/v1/blocks/latest" 2>/dev/null \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"].get("height") or "")' 2>/dev/null || echo)"
    restarts="$(docker inspect "$cid" --format '{{.RestartCount}}' 2>/dev/null || echo)"
    echo "$ts,$node,$rss,$fds,$data,$height,$restarts"
  done
}

trap '"$HERE/netem.sh" lan >/dev/null || true; [[ -n "${load_pid:-}" ]] && kill "$load_pid" 2>/dev/null || true' EXIT
python3 "$HERE/check_consensus.py" --rpc "$ALL_RPCS" --min-reachable 5
"$HERE/netem.sh" "$profile"
{
  echo "recorded_utc: $(date -u +%FT%TZ)"
  echo "git: $(git rev-parse HEAD)$([[ -z "$(git status --porcelain)" ]] || echo ' (uncommitted changes)')"
  echo "image: $(docker image inspect miraset-chain:devnet --format '{{.Id}}')"
  echo "cpu: $(lscpu | sed -n 's/^Model name: *//p'), cpus: $(nproc), memory: $(free -h | awk '/^Mem:/ {print $2}')"
  echo "kernel: $(uname -r), docker: $(docker version --format '{{.Server.Version}}')"
  echo "soak: duration=${secs}s rate=$rate/s senders=$senders interval=${interval}s profile=$profile"
} >"$out/environment.txt"

echo "ts,node,rss_bytes,fds,data_bytes,height,restarts" >"$out/samples.csv"
$COMPOSE run --rm -T loadgen loadgen "${IN_NETWORK_RPCS[@]}" \
  --faucet-key /keys/faucet/faucet.key --coordinator-key /keys/coordinator/coordinator.key \
  --senders "$senders" --rate "$rate" \
  --duration-secs "$secs" --warmup-secs "$warmup" --fund-amount "$fund" \
  >"$out/load.json" 2>"$out/load.log" &
load_pid=$!
echo "soak running for ${secs}s at $rate/s; samples every ${interval}s -> $out/samples.csv"
restarts_seen=0
while kill -0 "$load_pid" 2>/dev/null; do
  rows="$(sample)"
  echo "$rows" >>"$out/samples.csv"
  # A restarted container gets a fresh network namespace without the netem qdisc.
  restarts_now="$(awk -F, '{s += $7} END {print s + 0}' <<<"$rows")"
  if [[ "$restarts_now" != "$restarts_seen" && "$profile" != lan ]]; then
    echo "$(date -u +%FT%TZ) container restart seen; re-applying netem $profile" | tee -a "$out/events.log"
    "$HERE/netem.sh" "$profile" >>"$out/events.log" 2>&1 || true
  fi
  restarts_seen="$restarts_now"
  sleep "$interval"
done
load_status=0
wait "$load_pid" || load_status=$?
load_pid=""
sample >>"$out/samples.csv"

# Clear shaping first: a per-height check through netem would take hours.
"$HERE/netem.sh" lan >/dev/null || true
consensus_status=0
python3 "$HERE/check_consensus.py" --rpc "$ALL_RPCS" --min-reachable 5 >"$out/consensus.txt" 2>&1 \
  || consensus_status=$?
halts="$($COMPOSE logs --no-color node0 node1 node2 node3 node4 2>/dev/null | grep -ci 'safety halt' || true)"
echo "safety halt log lines: $halts" >>"$out/consensus.txt"

python3 "$HERE/soak_summary.py" "$out" | tee "$out/summary.md"
echo "loadgen exit: $load_status, consensus check exit: $consensus_status, safety halts: $halts"
[[ "$load_status" == 0 && "$consensus_status" == 0 && "$halts" == 0 ]]
