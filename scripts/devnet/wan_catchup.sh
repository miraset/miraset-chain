#!/usr/bin/env bash
# Catch-up under WAN load (TODO 8.5.3 finding 37): the stress-test devnet with a netem profile on
# every validator's egress and a fixed offered rate. node4 is paused (`docker pause`: frozen, its
# connections and netem shaping kept) for STOP_SECS while the others commit, then resumed; every
# 5 s its committed height and the highest one are recorded until it is within MAX_LAG blocks
# (caught up) or CATCHUP_SECS pass. At the end, with shaping cleared: agreement at every height,
# node logs, sync session lines (`EndSync`, vendored patch #18) and blacklistings
# (`BlacklistSyncServer`, patch #17) per node.
# Usage: [NODE_ARGS="--flag=v ..."] [EXTRA_COMPOSE=file.yml] scripts/devnet/wan_catchup.sh OUT_DIR PROFILE RATE  (repo root)
#   e.g. scripts/devnet/wan_catchup.sh ../runs/catchup wan-nojitter 1000
# Takes about 6 min plus funding. Raw outputs belong outside the repository.
set -uo pipefail
R="$(mkdir -p "$1" && cd "$1" && pwd)"
PROFILE="${2:?profile: a netem.sh profile (lan, wan, wan-far, wan-nojitter, wan-far-nojitter, wan-noloss)}"
RATE="${3:?offered rate in tx/s (0: no load)}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STOP_SECS="${STOP_SECS:-60}" CATCHUP_SECS="${CATCHUP_SECS:-240}" MAX_LAG="${MAX_LAG:-3}"
OVERRIDE=scripts/devnet/stress-override.yml
if [[ -n "${NODE_ARGS:-}" ]]; then
  OVERRIDE="$R/override.yml"
  extra="$(printf ', %s' $NODE_ARGS)"
  case "$extra" in
    *'|'* | *'&'* | *'\'*) echo "NODE_ARGS may not contain | & or \\" >&2; exit 64 ;;
  esac
  sed "s|--min-block-interval-ms=1000\]|--min-block-interval-ms=1000${extra}]|" scripts/devnet/stress-override.yml > "$OVERRIDE"
  [[ "$(grep -cF -- "--min-block-interval-ms=1000${extra}]" "$OVERRIDE")" == 5 ]] \
    || { echo "stress-override.yml format changed: node arguments not applied" >&2; exit 1; }
fi
# EXTRA_COMPOSE: one more compose file layered last (e.g. per-node sysctls).
C="docker compose -p miraset-measure -f docker-compose.yml -f $OVERRIDE${EXTRA_COMPOSE:+ -f $EXTRA_COMPOSE}"
export COMPOSE="$C"
LOAD_SECS=$(( 40 + STOP_SECS + CATCHUP_SECS )) SENDERS=256
RPCS="http://127.0.0.1:8545,http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548,http://127.0.0.1:8549"
log() { echo "[$(date -u +%T)] $*" | tee -a "$R/run.log"; }
height() { python3 -I -c 'import json,sys,urllib.request
try: print(json.load(urllib.request.urlopen(sys.argv[1]+"/v1/blocks/latest",timeout=2))["data"]["height"])
except Exception: print(-1)' "$1"; }
best() { local b=-1 h; for p in 8545 8546 8547 8548; do h=$(height "http://127.0.0.1:$p"); (( h > b )) && b=$h; done; echo "$b"; }

$C --profile observe down -v >/dev/null 2>&1
$C --profile observe build >/dev/null 2>&1; log "image built ($(git rev-parse --short HEAD)$([[ -z "$(git status --porcelain)" ]] || echo ', uncommitted changes'))"
$C --profile observe up -d >/dev/null 2>&1; sleep 25; log "devnet up (node args: ${NODE_ARGS:-none})"
"$HERE/netem.sh" "$PROFILE" | tee -a "$R/run.log"
# Every 5 s: each validator's outbound P2P sockets (`ss -tin`, dport 9948): send queue, unsent
# bytes, cwnd, RTT and retransmits per peer, into tcp-nodeN.txt; node IPs in ips.txt.
for n in 0 1 2 3 4; do echo "node$n $(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "miraset-measure-node$n-1")"; done > "$R/ips.txt"
( while true; do
    for n in 0 1 2 3 4; do
      { echo "@ $(date +%s)"; docker run --rm --network "container:miraset-measure-node$n-1" --entrypoint ss miraset-nettools:local \
          -tinH state established '( dport = :9948 )'; } >> "$R/tcp-node$n.txt" 2>/dev/null &
    done
    wait; sleep 5
  done ) & tcp_sampler=$!
trap 'kill $tcp_sampler 2>/dev/null; "$HERE/netem.sh" lan >/dev/null 2>&1' EXIT
trap 'echo "interrupted; stopping the devnet" >&2; $C unpause node4 >/dev/null 2>&1; $C --profile observe down -v >/dev/null 2>&1; exit 130' INT TERM

pid=""
if [[ "$RATE" != 0 ]]; then
  fund=$(( RATE * LOAD_SECS / SENDERS * 22 + 1000 ))
  log "rate $RATE/s: funding $SENDERS senders"
  # node4 is left out: it is paused during the run.
  $C run --rm -T loadgen loadgen --rpc http://node0:8545 --rpc http://node1:8545 --rpc http://node2:8545 --rpc http://node3:8545 \
    --faucet-key /keys/faucet/faucet.key --senders "$SENDERS" --rate "$RATE" --receipt-share 0 \
    --duration-secs "$LOAD_SECS" --warmup-secs 20 --drain-timeout-secs 60 --fund-amount "$fund" \
    > "$R/load.json" 2> "$R/load.log" &
  pid=$!
  for _ in $(seq 1 300); do grep -q "senders funded" "$R/load.log" 2>/dev/null && break; kill -0 $pid 2>/dev/null || break; sleep 1; done
  grep -q "senders funded" "$R/load.log" || { log "funding not confirmed"; }
fi
sleep 40
log "pause node4 at height $(height http://127.0.0.1:8549) (best $(best))"
$C pause node4 >/dev/null 2>&1
sleep "$STOP_SECS"
$C unpause node4 >/dev/null 2>&1
start=$(date +%s); caught="" h=-1 b=-1
log "node4 resumed (best $(best))"
echo "secs node4 best node0 node1 node2 node3" > "$R/catchup.txt"
while (( $(date +%s) - start < CATCHUP_SECS )); do
  h=$(height http://127.0.0.1:8549); b=$(best); t=$(( $(date +%s) - start ))
  echo "$t $h $b $(for p in 8545 8546 8547 8548; do height "http://127.0.0.1:$p"; done | tr '\n' ' ')" >> "$R/catchup.txt"
  if [[ -z "$caught" ]] && (( h >= 0 && b - h <= MAX_LAG )); then caught=$t; log "node4 caught up after ${t}s (height $h, best $b)"; fi
  sleep 5
done
[[ -n "$caught" ]] || log "node4 NOT caught up after ${CATCHUP_SECS}s (height $h, best $b)"
[[ -z "$pid" ]] || { wait $pid; log "loadgen exit $?"; }

kill $tcp_sampler 2>/dev/null
for n in 0 1 2 3 4; do # cumulative counters, including the send delays by message kind
  python3 -I -c 'import sys,urllib.request
try: sys.stdout.write(urllib.request.urlopen(sys.argv[1],timeout=5).read().decode())
except Exception as e: print("#", e)' "http://127.0.0.1:$((8545 + n))/metrics" > "$R/metrics-node$n.txt"
done
"$HERE/netem.sh" lan >/dev/null
python3 "$HERE/check_consensus.py" --rpc "$RPCS" --min-reachable 5 > "$R/agree.txt" 2>&1
log "agreement exit $?"
for n in 0 1 2 3 4; do
  docker logs "miraset-measure-node$n-1" > "$R/node$n.log" 2>&1
  log "node$n: height $(height "http://127.0.0.1:$((8545 + n))"), EndSync $(grep -c EndSync "$R/node$n.log"), BlacklistSyncServer $(grep -c BlacklistSyncServer "$R/node$n.log")"
done
$C --profile observe down -v >/dev/null 2>&1
log done
