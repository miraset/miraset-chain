#!/bin/bash

# Miraset Chain Demo Script
# This script demonstrates the full functionality of the blockchain

echo "====================================="
echo "Miraset Chain Full Functionality Demo"
echo "====================================="
echo ""

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Fixed genesis credentials for devnet
GENESIS_ADDR="4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd"
GENESIS_SECRET="0101010101010101010101010101010101010101010101010101010101010101"

# Cleanup function
cleanup_wallet() {
    echo -e "${YELLOW}Cleaning up old wallet for fresh test...${NC}"
    rm -f ~/.miraset/wallet.json
    echo ""
}

# Option to clean wallet
if [ "$1" == "--clean" ]; then
    cleanup_wallet
fi

echo -e "${BLUE}Step 1: Import Genesis Account${NC}"
# Check if genesis already exists, if not import it
if cargo run --bin miraset -- wallet list 2>/dev/null | grep -q "genesis"; then
    echo "Genesis account already exists, skipping import"
else
    cargo run --bin miraset -- wallet import genesis "$GENESIS_SECRET"
fi
echo ""

echo -e "${BLUE}Step 2: Check Genesis Balance${NC}"
cargo run --bin miraset -- wallet balance genesis
echo ""

echo -e "${BLUE}Step 3: Create Two Test Accounts${NC}"
# Check and create alice
if cargo run --bin miraset -- wallet list 2>/dev/null | grep -q "alice"; then
    echo "Alice account already exists, skipping creation"
else
    cargo run --bin miraset -- wallet new alice
fi

# Check and create bob
if cargo run --bin miraset -- wallet list 2>/dev/null | grep -q "bob"; then
    echo "Bob account already exists, skipping creation"
else
    cargo run --bin miraset -- wallet new bob
fi
echo ""

echo -e "${BLUE}Step 4: List All Accounts${NC}"
cargo run --bin miraset -- wallet list
echo ""

echo -e "${BLUE}Step 5: Get Alice's Address${NC}"
ALICE_ADDR=$(cargo run --bin miraset -- wallet list 2>/dev/null | grep alice | awk '{print $3}')
echo "Alice address: $ALICE_ADDR"
echo ""

echo -e "${BLUE}Step 6: Transfer 1000 tokens from Genesis to Alice${NC}"
cargo run --bin miraset -- wallet transfer genesis "$ALICE_ADDR" 1000
sleep 6 # Wait for block
echo ""

echo -e "${BLUE}Step 7: Check Alice's Balance${NC}"
cargo run --bin miraset -- wallet balance alice
echo ""

echo -e "${BLUE}Step 8: Send Chat Message from Alice${NC}"
cargo run --bin miraset -- chat send alice "Hello from Alice!"
sleep 6 # Wait for block
echo ""

echo -e "${BLUE}Step 9: Send Another Chat Message${NC}"
cargo run --bin miraset -- chat send alice "This is the Miraset blockchain!"
sleep 6
echo ""

echo -e "${BLUE}Step 10: List All Chat Messages${NC}"
cargo run --bin miraset -- chat list
echo ""

echo -e "${BLUE}Step 11: Check Latest Block via RPC${NC}"
echo "Raw JSON response:"
curl -s http://127.0.0.1:9944/block/latest
echo ""
echo ""

echo -e "${BLUE}Step 12: Check Balance via RPC${NC}"
echo "Alice's balance via RPC:"
curl -s "http://127.0.0.1:9944/balance/$ALICE_ADDR"
echo ""
echo ""

echo -e "${GREEN}====================================="
echo "Demo Complete!"
echo "=====================================${NC}"
echo ""
echo "Summary:"
echo "- Created and managed wallets"
echo "- Transferred tokens between accounts"
echo "- Sent chat messages"
echo "- Queried blockchain state via RPC"
echo ""
echo "The Miraset blockchain is fully functional with:"
echo "  ✓ Wallet management (create, import, export, list)"
echo "  ✓ Token transfers with signature verification"
echo "  ✓ Chat functionality"
echo "  ✓ Block production (every 5 seconds)"
echo "  ✓ RPC API for querying state"
echo "  ✓ Event system for tracking all activities"
echo ""
