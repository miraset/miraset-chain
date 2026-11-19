#!/usr/bin/env bash
# WAN under load (TODO 8.5.3; open since 8.5.2 F2/F5): the stress-test devnet with a netem
# profile on every validator's egress, then fixed offered rates one after another (transfers only,
# one loadgen per rate). Per rate: Prometheus over the last 90 s of load. At the end, with shaping
# cleared: agreement at every height, node logs, and counts of view timeouts and sync-server
# blacklistings (`BlacklistSyncServer`, vendored patch #17) per node.
# Usage: [NODE_ARGS="--flag=v ..."] [EXTRA_COMPOSE=file.yml] scripts/devnet/wan_load.sh OUT_DIR PROFILE "RATES"   (repo root)
# NODE_ARGS are appended to every validator's command line (stress-override.yml); a rate of 0 is
# an idle step (no loadgen, same measurement window).
#   e.g. scripts/devnet/wan_load.sh ../runs/wan wan "1000 2500 5000 10000"
# Each rate takes about 3 min plus funding. Raw outputs belong outside the repository.
set -uo pipefail
R="$(mkdir -p "$1" && cd "$1" && pwd)"
PROFILE="${2:?profile: a netem.sh profile (lan, wan, wan-far, wan-nojitter, wan-far-nojitter, wan-noloss)}"
RATES="${3:?rates, e.g. \"1000 5000\"}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OVERRIDE=scripts/devnet/stress-override.yml
# MIN_BLOCK_INTERVAL_MS (default 1000, as the stress tests) replaces the strict block interval.
MIN_BLOCK_INTERVAL_MS="${MIN_BLOCK_INTERVAL_MS:-1000}"
if [[ -n "${NODE_ARGS:-}" || "$MIN_BLOCK_INTERVAL_MS" != 1000 ]]; then
  OVERRIDE="$R/override.yml"
  extra="$(printf ', %s' ${NODE_ARGS:-})"
  [[ -n "${NODE_ARGS:-}" ]] || extra=""
  case "$extra$MIN_BLOCK_INTERVAL_MS" in
    *'|'* | *'&'* | *'\'*) echo "NODE_ARGS may not contain | & or \\" >&2; exit 64 ;;
  esac
  sed "s|--min-block-interval-ms=1000\]|--min-block-interval-ms=${MIN_BLOCK_INTERVAL_MS}${extra}]|" scripts/devnet/stress-override.yml > "$OVERRIDE"
  [[ "$(grep -cF -- "--min-block-interval-ms=${MIN_BLOCK_INTERVAL_MS}${extra}]" "$OVERRIDE")" == 5 ]] \
    || { echo "stress-override.yml format changed: node arguments not applied" >&2; exit 1; }
fi
# EXTRA_COMPOSE: one more compose file layered last (e.g. RUST_LOG for the view trace).
C="docker compose -p miraset-measure -f docker-compose.yml -f $OVERRIDE${EXTRA_COMPOSE:+ -f $EXTRA_COMPOSE}"
export COMPOSE="$C"
LOAD_SECS=150 WARMUP_SECS=20 WINDOW_END_SECS=120 SENDERS=256
RPCS="http://127.0.0.1:8545,http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548,http://127.0.0.1:8549"
log() { echo "[$(date -u +%T)] $*" | tee -a "$R/run.log"; }

$C --profile observe down -v >/dev/null 2>&1
$C --profile observe build >/dev/null 2>&1; log "image built ($(git rev-parse --short HEAD)$([[ -z "$(git status --porcelain)" ]] || echo ', uncommitted changes'))"
$C --profile observe up -d >/dev/null 2>&1; sleep 25; log "devnet up (min block interval ${MIN_BLOCK_INTERVAL_MS} ms, node args: ${NODE_ARGS:-none})"
"$HERE/netem.sh" "$PROFILE" | tee -a "$R/run.log"
"$HERE/netem.sh" show > "$R/netem.txt"
( while true; do
    ts=$(date +%s)
    docker stats --no-stream --format "$ts {{.Name}} {{.CPUPerc}} {{.MemUsage}}" | grep node >> "$R/stats.txt"
    for n in 0 1 2 3 4; do docker inspect -f "$ts node$n {{.State.Status}} oom={{.State.OOMKilled}} restarts={{.RestartCount}}" "miraset-measure-node$n-1"; done >> "$R/state.txt" 2>&1
    sleep 10
  done ) & sampler=$!
trap 'kill $sampler 2>/dev/null; "$HERE/netem.sh" lan >/dev/null 2>&1' EXIT
trap 'echo "interrupted; stopping the devnet" >&2; $C --profile observe down -v >/dev/null 2>&1; exit 130' INT TERM

for rate in $RATES; do
  if [[ "$rate" == 0 ]]; then # idle step: no load, same measurement window
    log "rate 0/s: idle"; sleep "$WINDOW_END_SECS"
    python3 -I "$HERE/prom_query.py" 90s > "$R/prom-0.json" 2>> "$R/run.log"
    continue
  fi
  fund=$(( rate * LOAD_SECS / SENDERS * 22 + 1000 ))
  log "rate $rate/s: funding $SENDERS senders"
  $C run --rm -T loadgen loadgen --rpc http://node0:8545 --rpc http://node1:8545 --rpc http://node2:8545 --rpc http://node3:8545 --rpc http://node4:8545 \
    --faucet-key /keys/faucet/faucet.key --senders "$SENDERS" --rate "$rate" --receipt-share 0 \
    --duration-secs "$LOAD_SECS" --warmup-secs "$WARMUP_SECS" --drain-timeout-secs 60 --fund-amount "$fund" \
    > "$R/rate-$rate.json" 2> "$R/rate-$rate.log" &
  pid=$!
  for _ in $(seq 1 300); do grep -q "senders funded" "$R/rate-$rate.log" 2>/dev/null && break; kill -0 $pid 2>/dev/null || break; sleep 1; done
  if grep -q "senders funded" "$R/rate-$rate.log"; then
    log "rate $rate/s: load running"
    sleep "$WINDOW_END_SECS"
    python3 -I "$HERE/prom_query.py" 90s > "$R/prom-$rate.json" 2>> "$R/run.log"
  else
    log "rate $rate/s: funding not confirmed"
  fi
  wait $pid; log "rate $rate/s: loadgen exit $?"
done

kill $sampler 2>/dev/null
"$HERE/netem.sh" lan >/dev/null
python3 "$HERE/check_consensus.py" --rpc "$RPCS" --min-reachable 5 > "$R/agree.txt" 2>&1
log "agreement exit $?"
for n in 0 1 2 3 4; do
  docker logs "miraset-measure-node$n-1" > "$R/node$n.log" 2>&1
  log "node$n: BlacklistSyncServer $(grep -c BlacklistSyncServer "$R/node$n.log"), warn/error lines $(grep -cE '"level":"(WARN|ERROR)"| (WARN|ERROR) ' "$R/node$n.log")"
done
$C --profile observe down -v >/dev/null 2>&1
log done
