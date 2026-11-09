#!/usr/bin/env bash
# Submit a signed faucet transfer to a devnet node and print the RPC response.
# Usage: scripts/devnet/transfer.sh <to> <amount> [rpc_url] [via_service]
set -euo pipefail

TO="${1:?recipient address required}"
AMOUNT="${2:?amount required}"
RPC="${3:-http://127.0.0.1:8545}"
SERVICE="${4:-node0}"
COMPOSE="${COMPOSE:-docker compose}"

faucet=$($COMPOSE exec -T "$SERVICE" miraset-chain pubkey --key-file /devnet/faucet.key)
account=$(curl -fsS "$RPC/account/$faucet")
nonce=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["nonce"])' <<<"$account")

tx=$($COMPOSE exec -T "$SERVICE" miraset-chain sign-transfer \
  --key-file /devnet/faucet.key --to "$TO" --amount "$AMOUNT" --nonce "$nonce")
curl -fsS -X POST -H 'content-type: application/json' --data "$tx" "$RPC/tx/submit"
echo
