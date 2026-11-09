#!/usr/bin/env bash
# Run crash / hang / partition / SIGKILL / resync scenarios against the 5-validator docker
# compose devnet (f = 1).
# After each fault: the remaining quorum must keep committing and all reachable nodes must agree
# at every height. After each recovery: the faulted node must catch up.
#
# Usage: scripts/devnet/fault_suite.sh   (from the repo root, with `docker compose up -d` running)
set -euo pipefail

COMPOSE="${COMPOSE:-docker compose}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="python3 $HERE/check_consensus.py"
ALL_RPCS="http://127.0.0.1:8545,http://127.0.0.1:8546,http://127.0.0.1:8547,http://127.0.0.1:8548,http://127.0.0.1:8549"
PROGRESS_WINDOW="${PROGRESS_WINDOW:-45}"
NETWORK="$($COMPOSE ps --format json node0 | python3 -c 'import json,sys; d=json.loads(sys.stdin.readline()); print(d["Networks"].split(",")[0])')"

rpcs_without() { # $1 = index of the faulted node
  local out=() i
  for i in 0 1 2 3 4; do
    [[ "$i" == "$1" ]] || out+=("http://127.0.0.1:$((8545 + i))")
  done
  (IFS=,; echo "${out[*]}")
}

step() { printf '\n=== %s ===\n' "$*"; }

volume_at() { # $1 = service, $2 = mount path -> docker volume name
  docker inspect "$($COMPOSE ps -q "$1")" \
    --format "{{range .Mounts}}{{if eq .Destination \"$2\"}}{{.Name}}{{end}}{{end}}"
}

expect_quorum_progress() { # $1 = faulted node index
  $CHECK --rpc "$(rpcs_without "$1")" --min-reachable 4 \
    --progress-window "$PROGRESS_WINDOW" --min-progress 2
}

expect_full_recovery() {
  $CHECK --rpc "$ALL_RPCS" --min-reachable 5 --progress-window "$PROGRESS_WINDOW" --min-progress 2 \
    --max-lag "${MAX_LAG:-5}"
}

step "key isolation: each validator container can read only its own key"
for i in 0 1 2 3 4; do
  found="$($COMPOSE exec -T "node$i" sh -c \
    'find / \( -path /proc -o -path /sys \) -prune -o -name "*.key" -print 2>/dev/null; true' | sort)"
  if [[ "$found" != "/var/lib/miraset-key/node$i.key" ]]; then
    echo "FAULT SUITE FAILED: node$i sees key files: ${found:-<none>}"
    exit 1
  fi
done

step "baseline: all 5 validators commit and agree"
"$HERE/transfer.sh" bob 1000 >/dev/null
expect_full_recovery

step "crash: docker compose stop node4"
$COMPOSE stop node4
"$HERE/transfer.sh" bob 1000 >/dev/null
expect_quorum_progress 4
step "recover: restart node4 from its own volume and catch up"
$COMPOSE start node4
expect_full_recovery

step "hang: docker compose pause node3"
$COMPOSE pause node3
expect_quorum_progress 3
step "recover: unpause node3"
$COMPOSE unpause node3
expect_full_recovery

step "partition: disconnect node2 from the devnet network"
docker network disconnect "$NETWORK" "$($COMPOSE ps -q node2)"
expect_quorum_progress 2
step "heal: reconnect node2"
docker network connect --alias node2 "$NETWORK" "$($COMPOSE ps -q node2)"
expect_full_recovery

step "sigkill: docker compose kill node1 (no graceful shutdown)"
$COMPOSE kill -s SIGKILL node1
expect_quorum_progress 1
step "recover: start node1; its signing guard keeps the votes signed before the kill"
$COMPOSE start node1
expect_full_recovery

# Vote signing guard (TODO 8.3.2): chain data is disposable, the guard is not.
step "resync: delete node1 chain data volume, keep its signing guard volume"
DATA_VOLUME="$(volume_at node1 /var/lib/miraset)"
GUARD_VOLUME="$(volume_at node1 /var/lib/miraset-guard)"
$COMPOSE rm -sf node1
docker volume rm "$DATA_VOLUME" >/dev/null
docker volume inspect "$GUARD_VOLUME" >/dev/null
$COMPOSE up -d node1
expect_quorum_progress 1
step "recover: node1 resyncs from genesis and catches up"
expect_full_recovery

step "no safety halts in any validator log"
if $COMPOSE logs node0 node1 node2 node3 node4 | grep -q "SAFETY HALT"; then
  $COMPOSE logs node0 node1 node2 node3 node4 | grep "SAFETY HALT"
  echo "FAULT SUITE FAILED: safety halt"
  exit 1
fi

step "final: every height agrees on all 5 validators"
$CHECK --rpc "$ALL_RPCS" --min-reachable 5 --max-lag "${MAX_LAG:-5}"
echo "FAULT SUITE PASSED"
