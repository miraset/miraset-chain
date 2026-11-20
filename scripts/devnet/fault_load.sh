#!/usr/bin/env bash
# Faults under load (TODO 8.5.3, measurement plan step 5 / §4.3): the stress-test devnet with a
# netem profile, a fixed offered rate from the load generator, and `fault_suite.sh` run while the
# load is on. Every 2 s the committed height of each validator is recorded (heights.txt), and every
# fault step is timestamped (faults.log); `fault_summary.py` turns them into the per-fault report
# (commit gap during the fault, time for the faulted node to catch up). A container that restarts
# loses its qdisc, so a watcher re-applies the profile to any running node without it. At the end:
# loadgen accounting (load.json: committed, pending at end = items lost), agreement at every
# height with shaping cleared, logs, and a scan for safety halts.
# Usage: [PROGRESS_WINDOW=s] [LOAD_SECS=s] scripts/devnet/fault_load.sh OUT_DIR PROFILE RATE  (repo root)
#   e.g. scripts/devnet/fault_load.sh ../runs/faults-lan lan 5000
# The plan's rate is half of the measured saturation (LAN ≈ 11.6k tx/s, wan-nojitter ≈ 4.4k tx/s).
# Takes about LOAD_SECS (default 1500 s) plus funding. Raw outputs belong outside the repository.
set -uo pipefail
R="$(mkdir -p "$1" && cd "$1" && pwd)"
PROFILE="${2:?profile: a netem.sh profile (lan, wan, wan-far, wan-nojitter, wan-far-nojitter, wan-noloss)}"
RATE="${3:?offered rate in tx/s}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOAD_SECS="${LOAD_SECS:-1500}" SENDERS=256
C="docker compose -p miraset-measure -f docker-compose.yml -f scripts/devnet/stress-override.yml"
export COMPOSE="$C"
RPCS="http://127.0.0.1:8545,http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548,http://127.0.0.1:8549"
log() { echo "[$(date -u +%T)] $*" | tee -a "$R/run.log"; }

$C --profile observe down -v >/dev/null 2>&1
$C --profile observe build >/dev/null 2>&1; log "image built ($(git rev-parse --short HEAD)$([[ -z "$(git status --porcelain)" ]] || echo ', uncommitted changes'))"
$C --profile observe up -d >/dev/null 2>&1; sleep 25; log "devnet up, profile $PROFILE, rate $RATE/s"
"$HERE/netem.sh" "$PROFILE" | tee -a "$R/run.log"

( while true; do # heights: epoch seconds, then node0..node4 (-1 when unreachable)
    line="$(date +%s.%N | cut -c1-14)"
    for p in 8545 8546 8547 8548 8549; do
      line+=" $(curl -fsS --max-time 1 "http://127.0.0.1:$p/v1/blocks/latest" 2>/dev/null \
        | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["data"]["height"])' 2>/dev/null || echo -1)"
    done
    echo "$line" >> "$R/heights.txt"; sleep 2
  done ) & heights=$!
watcher=""
if [[ "$PROFILE" != lan ]]; then
  ( while true; do # re-apply the profile to a running node that lost it (restart, re-create)
      for n in 0 1 2 3 4; do
        cid="$($C ps -q --status running "node$n" 2>/dev/null)"
        [[ -n "$cid" ]] || continue
        docker run --rm --network "container:$cid" --cap-add NET_ADMIN miraset-nettools:local qdisc show dev eth0 2>/dev/null \
          | grep -q netem && continue
        NETEM_NODES="node$n" "$HERE/netem.sh" "$PROFILE" >/dev/null 2>&1 && echo "[$(date -u +%T)] netem re-applied to node$n" >> "$R/run.log"
      done
      sleep 5
    done ) & watcher=$!
fi
trap 'kill $heights $watcher 2>/dev/null; "$HERE/netem.sh" lan >/dev/null 2>&1' EXIT
trap 'echo "interrupted; stopping the devnet" >&2; $C --profile observe down -v >/dev/null 2>&1; exit 130' INT TERM

fund=$(( RATE * LOAD_SECS / SENDERS * 22 + 1000 ))
log "funding $SENDERS senders"
$C run --rm -T loadgen loadgen --rpc http://node0:8545 --rpc http://node1:8545 --rpc http://node2:8545 --rpc http://node3:8545 --rpc http://node4:8545 \
  --faucet-key /keys/faucet/faucet.key --senders "$SENDERS" --rate "$RATE" --receipt-share 0 \
  --duration-secs "$LOAD_SECS" --warmup-secs 20 --drain-timeout-secs 120 --fund-amount "$fund" \
  > "$R/load.json" 2> "$R/load.log" &
pid=$!
for _ in $(seq 1 300); do grep -q "senders funded" "$R/load.log" 2>/dev/null && break; kill -0 $pid 2>/dev/null || break; sleep 1; done
grep -q "senders funded" "$R/load.log" || { log "funding not confirmed; stopping"; kill $pid 2>/dev/null; $C --profile observe down -v >/dev/null 2>&1; exit 1; }
log "load running; fault suite starts in 30 s"
sleep 30
"$HERE/fault_suite.sh" 2>&1 | while IFS= read -r line; do echo "$(date +%s.%N | cut -c1-14) $line"; done > "$R/faults.log"
suite=${PIPESTATUS[0]}
log "fault suite exit $suite ($(grep -c '^[0-9.]* === ' "$R/faults.log") steps)"
wait $pid; log "loadgen exit $?"

kill $heights $watcher 2>/dev/null
"$HERE/netem.sh" lan >/dev/null
python3 "$HERE/check_consensus.py" --rpc "$RPCS" --min-reachable 5 > "$R/agree.txt" 2>&1
log "agreement exit $?"
for n in 0 1 2 3 4; do docker logs "miraset-measure-node$n-1" > "$R/node$n.log" 2>&1; done
log "safety halts: $(cat "$R"/node*.log | grep -ci 'safety halt')"
python3 -I "$HERE/fault_summary.py" "$R" | tee "$R/summary.txt"
$C --profile observe down -v >/dev/null 2>&1
log done
