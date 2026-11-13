#!/usr/bin/env bash
# Stress test (TODO 8.5.3): 4 loadgens x 256 senders x 7 500 tx/s, started 150 s apart, so the
# offered load steps 7.5k -> 15k -> 22.5k -> 30k tx/s (transfers only), 150 s per step.
# Funding happens before any load (--start-delay-secs) and retries 429/503; the first run
# (2026-11-13) funded at step start and stopped at 15k tx/s when mempools were full.
# Usage: scripts/devnet/stress.sh OUT_DIR   (run from the repository root; needs ~15 min,
# ~10 GB free host RAM and ~25 GB disk). Each node is limited to 2.5 GiB (stress-override.yml).
set -uo pipefail
R="$(mkdir -p "$1" && cd "$1" && pwd)"
C="docker compose -p miraset-measure -f docker-compose.yml -f scripts/devnet/stress-override.yml"
log() { echo "[$(date -u +%T)] $*" | tee -a "$R/run.log"; }
net() { for n in 0 1 2 3 4; do docker exec "miraset-measure-node$n-1" cat /proc/net/dev 2>/dev/null | awk -v n=$n -v t="$(date +%s)" '/eth0/ {print n, $2, $10, t}'; done > "$1"; }
STEP=150
$C --profile observe down -v >/dev/null 2>&1
$C --profile observe build >/dev/null 2>&1; log "image built"
$C --profile observe up -d >/dev/null 2>&1; sleep 25; log "devnet up"
# Sampler: every 10 s, per-node CPU and memory, host memory, restarts / OOM.
( while true; do
    ts=$(date +%s)
    docker stats --no-stream --format "$ts {{.Name}} {{.CPUPerc}} {{.MemUsage}}" | grep node >> "$R/stats.txt"
    echo "$ts $(free -m | awk '/Mem:/ {print $3, $7}')" >> "$R/host_mem.txt"
    for n in 0 1 2 3 4; do docker inspect -f "$ts node$n {{.State.Status}} oom={{.State.OOMKilled}} restarts={{.RestartCount}}" "miraset-measure-node$n-1"; done >> "$R/state.txt" 2>&1
    sleep 10
  done ) & sampler=$!
fund=250000
# Every loadgen funds its senders first, strictly one after another (they share the faucet
# account and read its nonce when they start), then starts its load at its step:
# t0 + 90 s + i x STEP.
t0=$(date +%s)
for i in 0 1 2 3; do
  dur=$(( (4 - i) * STEP ))
  delay=$(( t0 + 90 + i * STEP - $(date +%s) ))
  (( delay < 0 )) && delay=0
  log "loadgen $i launched: funds now, +7500 tx/s from t0+$(( 90 + i * STEP ))s for ${dur}s"
  $C run --rm -T loadgen loadgen --rpc http://node0:8545 --rpc http://node1:8545 --rpc http://node2:8545 --rpc http://node3:8545 --rpc http://node4:8545 \
    --faucet-key /keys/faucet/faucet.key --senders 256 --rate 7500 --receipt-share 0 \
    --duration-secs "$dur" --warmup-secs 10 --drain-timeout-secs 60 --fund-amount "$fund" \
    --start-delay-secs "$delay" > "$R/lg$i.json" 2> "$R/lg$i.log" &
  pids[$i]=$!
  for _ in $(seq 1 120); do grep -q "senders funded" "$R/lg$i.log" 2>/dev/null && break; sleep 1; done
  grep -q "senders funded" "$R/lg$i.log" || log "loadgen $i: funding not confirmed after 120 s"
done
wait_s=$(( t0 + 90 - $(date +%s) ))
(( wait_s > 0 )) && sleep "$wait_s"
for i in 0 1 2 3; do
  net "$R/net_step${i}_a.txt"
  sleep $(( STEP - 60 ))
  sleep 60
  # Metrics over the last 90 s of the step (after ramp-up).
  net "$R/net_step${i}_b.txt"
  python3 -I scripts/devnet/prom_query.py 90s > "$R/prom_step$i.json" 2>> "$R/run.log"
  log "step $i done (offered $(( (i + 1) * 7500 )) tx/s)"
done
for i in 0 1 2 3; do wait ${pids[$i]}; log "loadgen $i exit $?"; done
python3 -I scripts/devnet/prom_query.py 60s > "$R/prom_drain.json" 2>> "$R/run.log"
kill $sampler
python3 scripts/devnet/check_consensus.py --rpc http://127.0.0.1:8545,http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548,http://127.0.0.1:8549 --min-reachable 5 > "$R/agree.txt" 2>&1
log "agreement exit $?"
for n in 0 1 2 3 4; do
  docker logs "miraset-measure-node$n-1" > "$R/node$n.log" 2>&1
done
docker system df -v 2>/dev/null | grep miraset-measure > "$R/volumes.txt"
$C --profile observe down -v >/dev/null 2>&1
log done
