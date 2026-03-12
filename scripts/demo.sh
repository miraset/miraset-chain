#!/usr/bin/env bash
# =============================================================================
# MIRASET DEMO Script
# =============================================================================
# Demonstrates the full end-to-end flow:
#   1. Start local devnet node
#   2. Create wallet accounts
#   3. Transfer tokens
#   4. Register a worker
#   5. Submit an inference job via coordinator
#   6. Show events and blocks
#
# Prerequisites:
#   - cargo build --workspace (or --release)
#   - Ports 9944 and 8080 available
#
# Usage:
#   ./scripts/demo.sh
# =============================================================================

set -e

MIRASET="cargo run --bin miraset --"
RPC="http://127.0.0.1:9944"
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

banner() { echo -e "\n${CYAN}═══════════════════════════════════════${NC}"; echo -e "${CYAN}  $1${NC}"; echo -e "${CYAN}═══════════════════════════════════════${NC}"; }
step()   { echo -e "${GREEN}▸ $1${NC}"; }
info()   { echo -e "${YELLOW}  $1${NC}"; }

# -------------------------------------------------------------------
banner "MIRASET DEMO — Building workspace"
cargo build --workspace 2>/dev/null
step "Build complete"

# -------------------------------------------------------------------
banner "Step 1: Start devnet node (background)"
$MIRASET node start --rpc-addr 127.0.0.1:9944 &
NODE_PID=$!
step "Node PID: $NODE_PID"
sleep 3

# Check node health
if curl -s $RPC/health | grep -q "healthy"; then
    step "Node is healthy ✓"
else
    echo "ERROR: Node did not start"
    kill $NODE_PID 2>/dev/null
    exit 1
fi

# -------------------------------------------------------------------
banner "Step 2: Create wallet accounts"

$MIRASET wallet new alice 2>/dev/null || true
$MIRASET wallet new bob   2>/dev/null || true
step "Created accounts: alice, bob"

$MIRASET wallet list
echo ""

# -------------------------------------------------------------------
banner "Step 3: Check genesis balance"

GENESIS_ADDR=$(curl -s $RPC/health | python3 -c "import sys; print('OK')" 2>/dev/null && echo "checking..." || echo "checking...")
$MIRASET wallet balance alice --rpc $RPC 2>/dev/null || info "alice starts with 0 (needs funding from genesis)"

# Genesis account has a fixed key [1u8;32]
GENESIS_SECRET="0101010101010101010101010101010101010101010101010101010101010101"
$MIRASET wallet import genesis "$GENESIS_SECRET" 2>/dev/null || true
step "Imported genesis account"

$MIRASET wallet balance genesis --rpc $RPC
echo ""

# -------------------------------------------------------------------
banner "Step 4: Transfer tokens"

ALICE_ADDR=$($MIRASET wallet list 2>/dev/null | grep alice | awk '{print $NF}')
step "Alice address: $ALICE_ADDR"

$MIRASET wallet transfer genesis "$ALICE_ADDR" 1000000 --rpc $RPC
step "Transferred 1,000,000 SECCO from genesis to alice"

sleep 6  # Wait for block
$MIRASET wallet balance alice --rpc $RPC

# -------------------------------------------------------------------
banner "Step 5: Send chat message"

$MIRASET chat send alice "Hello from MIRASET demo!" --rpc $RPC
step "Chat message sent"
sleep 6

$MIRASET chat list --rpc $RPC --limit 5
echo ""

# -------------------------------------------------------------------
banner "Step 6: Check blocks and events"

step "Latest block:"
curl -s $RPC/block/latest | python3 -m json.tool 2>/dev/null || curl -s $RPC/block/latest
echo ""

step "Events:"
curl -s "$RPC/events?from_height=0&limit=10" | python3 -m json.tool 2>/dev/null || curl -s "$RPC/events?from_height=0&limit=10"
echo ""

# -------------------------------------------------------------------
banner "Step 7: Check epoch status"

curl -s $RPC/epoch | python3 -m json.tool 2>/dev/null || curl -s $RPC/epoch
echo ""

# -------------------------------------------------------------------
banner "Step 8: List workers"

curl -s $RPC/workers | python3 -m json.tool 2>/dev/null || curl -s $RPC/workers
echo ""

# -------------------------------------------------------------------
banner "Step 9: Submit inference job (via coordinator)"

step "Submitting job for alice..."
JOB_RESULT=$(curl -s -X POST $RPC/jobs/submit \
    -H "Content-Type: application/json" \
    -d "{\"requester\":\"$ALICE_ADDR\",\"model_id\":\"llama2\",\"max_tokens\":256,\"escrow_amount\":1000}")
echo "$JOB_RESULT" | python3 -m json.tool 2>/dev/null || echo "$JOB_RESULT"
echo ""

step "Jobs on chain:"
curl -s $RPC/jobs | python3 -m json.tool 2>/dev/null || curl -s $RPC/jobs
echo ""

# -------------------------------------------------------------------
banner "Demo complete!"
echo ""
echo "  Node:    $RPC"
echo "  PID:     $NODE_PID"
echo ""
echo "  To stop: kill $NODE_PID"
echo ""
echo "  To start the desktop wallet:"
echo "    cd wallet && bunx tauri dev"
echo ""
echo "  To start a worker:"
echo "    cargo run --bin miraset-worker"
echo ""

# Don't kill the node — let user explore
wait $NODE_PID 2>/dev/null || true

