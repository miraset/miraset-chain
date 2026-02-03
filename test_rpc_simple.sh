#!/bin/bash
# RPC Integration Tests

RPC_URL="http://127.0.0.1:9944"
GENESIS_ADDR="4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd"

echo "=== RPC Integration Tests ==="
echo ""

# Test 1: Get Balance
echo "Test 1: GET /balance/{address}"
BALANCE=$(curl -s $RPC_URL/balance/$GENESIS_ADDR)
echo "Balance: $BALANCE"
echo ""

# Test 2: Get Nonce
echo "Test 2: GET /nonce/{address}"
NONCE=$(curl -s $RPC_URL/nonce/$GENESIS_ADDR)
echo "Nonce: $NONCE"
echo ""

# Test 3: Get Latest Block
echo "Test 3: GET /block/latest"
echo "Response:"
curl -s $RPC_URL/block/latest
echo ""
echo ""

# Test 4: Get Genesis Block
echo "Test 4: GET /block/0"
echo "Response:"
curl -s $RPC_URL/block/0
echo ""
echo ""

# Test 5: Get Events
echo "Test 5: GET /events"
echo "Response:"
curl -s "$RPC_URL/events?from_height=0&limit=5"
echo ""
echo ""

# Test 6: Get Chat Messages
echo "Test 6: GET /chat/messages"
echo "Response:"
curl -s "$RPC_URL/chat/messages?limit=10"
echo ""
echo ""

# Test 7: Invalid Address
echo "Test 7: Invalid Address (should be 400)"
curl -s -o /dev/null -w "HTTP Status: %{http_code}\n" $RPC_URL/balance/invalid
echo ""

# Test 8: Nonexistent Block
echo "Test 8: Nonexistent Block (should be 404)"
curl -s -o /dev/null -w "HTTP Status: %{http_code}\n" $RPC_URL/block/999999
echo ""

echo "=== Tests Complete ==="
