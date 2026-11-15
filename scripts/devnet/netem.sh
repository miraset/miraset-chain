#!/usr/bin/env bash
# Apply or clear an emulated WAN profile on every validator's egress (TODO 8.5.3).
#
# Delay is added on each container's egress, so a round trip between two validators carries it
# twice. Jitter with a normal distribution reorders packets (netem default). The load generator
# is not shaped: its requests arrive undelayed, responses and commit events are delayed once.
# A restarted container loses its qdisc (soak.sh re-applies it).
# Profiles (per-egress values; round trip roughly doubles them):
#   lan      no shaping (clears any profile)
#   wan      25ms ± 5ms (normal), 0.1% loss   -> ~50ms RTT, one region / nearby regions
#   wan-far  75ms ± 15ms (normal), 0.5% loss  -> ~150ms RTT, intercontinental
#   wan-nojitter     25ms fixed, 0.1% loss    -> wan without reordering (netem jitter delays each
#   wan-far-nojitter 75ms fixed, 0.5% loss       packet independently, so it reorders bursts;
#                                                TCP reads that as loss, 8.5.3 WAN under load)
#   wan-noloss       25ms fixed, no loss      -> separates latency from loss-limited TCP throughput
#
# Usage: scripts/devnet/netem.sh <lan|wan|wan-far|wan-nojitter|wan-far-nojitter|wan-noloss|show>
# Needs a running devnet; builds the `miraset-nettools:local` sidecar image on first use.
set -euo pipefail

COMPOSE="${COMPOSE:-docker compose}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE=miraset-nettools:local
NODES=(node0 node1 node2 node3 node4)

profile_args() {
  case "$1" in
    # limit: netem's default 1000-packet queue would add drops of its own at high rates.
    wan) echo "limit 100000 delay 25ms 5ms distribution normal loss 0.1%" ;;
    wan-far) echo "limit 100000 delay 75ms 15ms distribution normal loss 0.5%" ;;
    wan-nojitter) echo "limit 100000 delay 25ms loss 0.1%" ;;
    wan-far-nojitter) echo "limit 100000 delay 75ms loss 0.5%" ;;
    wan-noloss) echo "limit 100000 delay 25ms" ;;
    *) echo "unknown profile: $1" >&2; exit 64 ;;
  esac
}

ensure_image() {
  docker image inspect "$IMAGE" >/dev/null 2>&1 \
    || docker build -q -t "$IMAGE" -f "$HERE/nettools.Dockerfile" "$HERE" >/dev/null
}

tc_in() { # $1 = service, rest = tc arguments
  local cid
  cid="$($COMPOSE ps -q "$1")"
  [[ -n "$cid" ]] || { echo "service $1 is not running" >&2; exit 1; }
  docker run --rm --network "container:$cid" --cap-add NET_ADMIN "$IMAGE" "${@:2}"
}

mode="${1:?usage: netem.sh <lan|wan|wan-far|wan-nojitter|wan-far-nojitter|wan-noloss|show>}"
ensure_image
for node in "${NODES[@]}"; do
  case "$mode" in
    show) printf '%s: ' "$node"; tc_in "$node" qdisc show dev eth0 | awk 'NR == 1' ;;
    lan) tc_in "$node" qdisc del dev eth0 root 2>/dev/null || true ;;
    *)
      # shellcheck disable=SC2046  # profile arguments are intentionally word-split
      tc_in "$node" qdisc replace dev eth0 root netem $(profile_args "$mode")
      ;;
  esac
done
[[ "$mode" == show ]] || echo "netem profile '$mode' applied to ${NODES[*]}"
