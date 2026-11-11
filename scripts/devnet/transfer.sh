#!/usr/bin/env bash
# Submit a signed faucet transfer to a devnet node and print the RPC response.
# Usage: scripts/devnet/transfer.sh <to> <amount> [rpc_url]
# The faucet key lives only in the `faucet-key` volume, mounted by the one-shot `faucet` tool
# container (no network); validator containers never see it.
set -euo pipefail

TO="${1:?recipient address required}"
AMOUNT="${2:?amount required}"
RPC="${3:-http://127.0.0.1:8545}"
COMPOSE="${COMPOSE:-docker compose}"
FAUCET_KEY=/keys/faucet/faucet.key
faucet_tool() { $COMPOSE run --rm -T faucet "$@"; }

faucet=$(faucet_tool pubkey --key-file "$FAUCET_KEY")
account=$(curl -fsS "$RPC/v1/accounts/$faucet")
nonce=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["nonce"])' <<<"$account")

tx=$(faucet_tool sign-transfer --key-file "$FAUCET_KEY" --to "$TO" --amount "$AMOUNT" --nonce "$nonce")
curl -fsS -X POST -H 'content-type: application/json' --data "$tx" "$RPC/v1/transactions"
echo
