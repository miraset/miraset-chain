# Miraset Chain - User Case Scenarios & Test Cases

This document describes comprehensive user scenarios and test cases for the Miraset Chain.

---

## Test Environment Setup

### Prerequisites
1. Node running on `http://127.0.0.1:9944`
2. Genesis account imported
3. Test accounts created

### Genesis Account (Devnet)
```
Address: 4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd
Secret:  0101010101010101010101010101010101010101010101010101010101010101
Balance: 1,000,000,000,000 MIRA
```

---

## User Case Scenarios

### Scenario 1: New User Onboarding

**Goal**: New user creates wallet, receives tokens, and makes first transaction

**Steps**:

1. **Create Account**
   ```bash
   cargo run --bin miraset -- wallet new alice
   ```
   **Expected**: 
   - ✅ New account created
   - ✅ Address displayed
   - ✅ Wallet file created at `~/.miraset/wallet.json`

2. **Check Initial Balance**
   ```bash
   cargo run --bin miraset -- wallet balance alice
   ```
   **Expected**: 
   - ✅ Balance: 0

3. **Request Tokens from Genesis**
   ```bash
   # Import genesis first
   cargo run --bin miraset -- wallet import genesis 0101010101010101010101010101010101010101010101010101010101010101
   
   # Get alice's address
   ALICE_ADDR=$(cargo run --bin miraset -- wallet list | grep alice | awk '{print $3}')
   
   # Transfer 10000 tokens
   cargo run --bin miraset -- wallet transfer genesis $ALICE_ADDR 10000
   ```
   **Expected**: 
   - ✅ Transaction submitted successfully
   - ✅ Message: "Transfer submitted: genesis -> [address], amount: 10000"

4. **Wait for Block Confirmation**
   ```bash
   sleep 6
   ```

5. **Verify Balance**
   ```bash
   cargo run --bin miraset -- wallet balance alice
   ```
   **Expected**: 
   - ✅ Balance: 10000

6. **Make First Transaction**
   ```bash
   cargo run --bin miraset -- wallet new bob
   BOB_ADDR=$(cargo run --bin miraset -- wallet list | grep bob | awk '{print $3}')
   cargo run --bin miraset -- wallet transfer alice $BOB_ADDR 1000
   sleep 6
   cargo run --bin miraset -- wallet balance bob
   ```
   **Expected**: 
   - ✅ Bob receives 1000 tokens
   - ✅ Alice balance reduced by 1000

---

### Scenario 2: Chat Communication

**Goal**: Users communicate via on-chain chat

**Steps**:

1. **Setup Two Accounts**
   ```bash
   cargo run --bin miraset -- wallet new alice
   cargo run --bin miraset -- wallet new bob
   
   # Fund accounts
   ALICE_ADDR=$(cargo run --bin miraset -- wallet list | grep alice | awk '{print $3}')
   BOB_ADDR=$(cargo run --bin miraset -- wallet list | grep bob | awk '{print $3}')
   
   cargo run --bin miraset -- wallet transfer genesis $ALICE_ADDR 5000
   sleep 6
   cargo run --bin miraset -- wallet transfer genesis $BOB_ADDR 5000
   sleep 6
   ```

2. **Alice Sends First Message**
   ```bash
   cargo run --bin miraset -- chat send alice "Hi Bob! Welcome to Miraset!"
   sleep 6
   ```
   **Expected**: 
   - ✅ Message submitted
   - ✅ Transaction included in block

3. **Bob Responds**
   ```bash
   cargo run --bin miraset -- chat send bob "Hi Alice! This is amazing!"
   sleep 6
   ```

4. **View Conversation**
   ```bash
   cargo run --bin miraset -- chat list
   ```
   **Expected**: 
   - ✅ Both messages visible
   - ✅ Messages in chronological order
   - ✅ Timestamps displayed
   - ✅ Sender addresses shown

5. **Verify via RPC**
   ```bash
   curl "http://127.0.0.1:9944/chat/messages?limit=10"
   ```
   **Expected**: 
   - ✅ JSON response with messages
   - ✅ Includes from, message, timestamp fields

---

### Scenario 3: Multiple Transfers (Stress Test)

**Goal**: Test system handling multiple concurrent transactions

**Steps**:

1. **Setup Multiple Accounts**
   ```bash
   for name in alice bob charlie dave eve; do
       cargo run --bin miraset -- wallet new $name
   done
   ```

2. **Fund All Accounts**
   ```bash
   for name in alice bob charlie dave eve; do
       ADDR=$(cargo run --bin miraset -- wallet list | grep $name | awk '{print $3}')
       cargo run --bin miraset -- wallet transfer genesis $ADDR 10000
       sleep 6
   done
   ```

3. **Execute Multiple Transfers**
   ```bash
   # Alice -> Bob
   BOB_ADDR=$(cargo run --bin miraset -- wallet list | grep bob | awk '{print $3}')
   cargo run --bin miraset -- wallet transfer alice $BOB_ADDR 1000 &
   
   # Charlie -> Dave
   DAVE_ADDR=$(cargo run --bin miraset -- wallet list | grep dave | awk '{print $3}')
   cargo run --bin miraset -- wallet transfer charlie $DAVE_ADDR 2000 &
   
   # Eve -> Alice
   ALICE_ADDR=$(cargo run --bin miraset -- wallet list | grep alice | awk '{print $3}')
   cargo run --bin miraset -- wallet transfer eve $ALICE_ADDR 500 &
   
   wait
   sleep 6
   ```

4. **Verify All Balances**
   ```bash
   for name in alice bob charlie dave eve; do
       cargo run --bin miraset -- wallet balance $name
   done
   ```
   **Expected**: 
   - ✅ Alice: 10000 - 1000 + 500 = 9500
   - ✅ Bob: 10000 + 1000 = 11000
   - ✅ Charlie: 10000 - 2000 = 8000
   - ✅ Dave: 10000 + 2000 = 12000
   - ✅ Eve: 10000 - 500 = 9500

---

### Scenario 4: Account Recovery

**Goal**: User exports and imports account on different machine

**Steps**:

1. **Create and Fund Account**
   ```bash
   cargo run --bin miraset -- wallet new alice
   ALICE_ADDR=$(cargo run --bin miraset -- wallet list | grep alice | awk '{print $3}')
   cargo run --bin miraset -- wallet transfer genesis $ALICE_ADDR 5000
   sleep 6
   ```

2. **Export Secret Key**
   ```bash
   cargo run --bin miraset -- wallet export alice
   ```
   **Expected**: 
   - ✅ Secret key displayed
   - ✅ Warning message shown
   
   **Save the secret key**: `SECRET_KEY=<displayed_key>`

3. **Simulate New Machine (Delete Wallet)**
   ```bash
   rm ~/.miraset/wallet.json
   ```

4. **Import Account**
   ```bash
   cargo run --bin miraset -- wallet import alice $SECRET_KEY
   ```
   **Expected**: 
   - ✅ Account imported with same address

5. **Verify Balance Preserved**
   ```bash
   cargo run --bin miraset -- wallet balance alice
   ```
   **Expected**: 
   - ✅ Balance: 5000 (unchanged)

---

### Scenario 5: TUI Interactive Session

**Goal**: Use TUI for complete workflow

**Steps**:

1. **Launch TUI**
   ```bash
   cargo run --bin miraset-tui
   ```

2. **Navigate to Wallet Tab**
   - Press `1`
   **Expected**: 
   - ✅ List of accounts displayed
   - ✅ Balances shown
   - ✅ Addresses visible

3. **Switch to Chat Tab**
   - Press `2`
   **Expected**: 
   - ✅ Message history displayed
   - ✅ Input box at bottom
   - ✅ Account selector visible

4. **Send Message**
   - Select account with `↑`/`↓`
   - Type message: "Testing TUI chat!"
   - Press `Enter`
   **Expected**: 
   - ✅ Message submitted
   - ✅ Confirmation shown

5. **Refresh Data**
   - Wait 6 seconds
   - Press `r`
   **Expected**: 
   - ✅ New message appears in history
   - ✅ Latest block height updated

6. **View Chain Info**
   - Press `3`
   **Expected**: 
   - ✅ Block height displayed
   - ✅ Recent blocks listed
   - ✅ Transaction count shown

7. **Exit**
   - Press `q`
   **Expected**: 
   - ✅ TUI closes cleanly

---

### Scenario 6: RPC Integration Testing

**Goal**: External application integrates with blockchain via RPC

**Test Script** (`test_rpc_integration.sh`):

```bash
#!/bin/bash

echo "=== RPC Integration Tests ==="

# Test 1: Get Balance
echo "Test 1: Get Balance"
RESPONSE=$(curl -s http://127.0.0.1:9944/balance/4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd)
if [ ! -z "$RESPONSE" ]; then
    echo "✅ Balance retrieved: $RESPONSE"
else
    echo "❌ Failed to get balance"
fi

# Test 2: Get Latest Block
echo -e "\nTest 2: Get Latest Block"
BLOCK=$(curl -s http://127.0.0.1:9944/block/latest)
HEIGHT=$(echo $BLOCK | jq -r '.height')
if [ ! -z "$HEIGHT" ]; then
    echo "✅ Latest block height: $HEIGHT"
else
    echo "❌ Failed to get latest block"
fi

# Test 3: Get Nonce
echo -e "\nTest 3: Get Nonce"
NONCE=$(curl -s http://127.0.0.1:9944/nonce/4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd)
if [ ! -z "$NONCE" ]; then
    echo "✅ Nonce retrieved: $NONCE"
else
    echo "❌ Failed to get nonce"
fi

# Test 4: Get Events
echo -e "\nTest 4: Get Events"
EVENTS=$(curl -s "http://127.0.0.1:9944/events?from_height=0&limit=5")
EVENT_COUNT=$(echo $EVENTS | jq '. | length')
echo "✅ Retrieved $EVENT_COUNT events"

# Test 5: Get Chat Messages
echo -e "\nTest 5: Get Chat Messages"
MESSAGES=$(curl -s "http://127.0.0.1:9944/chat/messages?limit=10")
MSG_COUNT=$(echo $MESSAGES | jq '. | length')
echo "✅ Retrieved $MSG_COUNT chat messages"

# Test 6: Get Block by Height
echo -e "\nTest 6: Get Block by Height"
GENESIS=$(curl -s http://127.0.0.1:9944/block/0)
GENESIS_HEIGHT=$(echo $GENESIS | jq -r '.height')
if [ "$GENESIS_HEIGHT" == "0" ]; then
    echo "✅ Genesis block retrieved"
else
    echo "❌ Failed to get genesis block"
fi

# Test 7: Invalid Address (Error Handling)
echo -e "\nTest 7: Invalid Address Error Handling"
STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:9944/balance/invalid)
if [ "$STATUS" == "400" ]; then
    echo "✅ Properly returns 400 for invalid address"
else
    echo "❌ Unexpected status code: $STATUS"
fi

# Test 8: Nonexistent Block (Error Handling)
echo -e "\nTest 8: Nonexistent Block Error Handling"
STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:9944/block/999999)
if [ "$STATUS" == "404" ]; then
    echo "✅ Properly returns 404 for nonexistent block"
else
    echo "❌ Unexpected status code: $STATUS"
fi

echo -e "\n=== All RPC Tests Complete ==="
```

**Expected Results**:
- ✅ All 8 tests pass
- ✅ Proper error handling for invalid inputs
- ✅ Fast response times (<10ms for local)

---

### Scenario 7: Nonce Management

**Goal**: Test proper nonce handling and replay protection

**Steps**:

1. **Get Current Nonce**
   ```bash
   ALICE_ADDR=$(cargo run --bin miraset -- wallet list | grep alice | awk '{print $3}')
   NONCE=$(curl -s http://127.0.0.1:9944/nonce/$ALICE_ADDR)
   echo "Current nonce: $NONCE"
   ```

2. **Send Transaction**
   ```bash
   cargo run --bin miraset -- wallet transfer alice $BOB_ADDR 100
   sleep 6
   ```

3. **Verify Nonce Incremented**
   ```bash
   NEW_NONCE=$(curl -s http://127.0.0.1:9944/nonce/$ALICE_ADDR)
   echo "New nonce: $NEW_NONCE"
   ```
   **Expected**: 
   - ✅ NEW_NONCE = NONCE + 1

4. **Test Multiple Sequential Transactions**
   ```bash
   for i in {1..5}; do
       cargo run --bin miraset -- wallet transfer alice $BOB_ADDR 10
       sleep 6
   done
   
   FINAL_NONCE=$(curl -s http://127.0.0.1:9944/nonce/$ALICE_ADDR)
   echo "Final nonce: $FINAL_NONCE"
   ```
   **Expected**: 
   - ✅ Nonce increases by 5

---

### Scenario 8: Event Tracking

**Goal**: Verify all events are properly emitted and queryable

**Steps**:

1. **Get Initial Event Count**
   ```bash
   EVENTS=$(curl -s "http://127.0.0.1:9944/events?from_height=0&limit=1000")
   INITIAL_COUNT=$(echo $EVENTS | jq '. | length')
   echo "Initial events: $INITIAL_COUNT"
   ```

2. **Perform Transfer**
   ```bash
   cargo run --bin miraset -- wallet transfer alice $BOB_ADDR 500
   sleep 6
   ```

3. **Check for Transfer Event**
   ```bash
   EVENTS=$(curl -s "http://127.0.0.1:9944/events?from_height=0&limit=1000")
   NEW_COUNT=$(echo $EVENTS | jq '. | length')
   
   TRANSFER_EVENT=$(echo $EVENTS | jq '.[] | select(.event_type == "Transferred") | select(.amount == 500)')
   ```
   **Expected**: 
   - ✅ NEW_COUNT = INITIAL_COUNT + 1
   - ✅ Transfer event contains correct from, to, amount

4. **Send Chat Message**
   ```bash
   cargo run --bin miraset -- chat send alice "Event test message"
   sleep 6
   ```

5. **Check for Chat Event**
   ```bash
   EVENTS=$(curl -s "http://127.0.0.1:9944/events?from_height=0&limit=1000")
   CHAT_EVENT=$(echo $EVENTS | jq '.[] | select(.event_type == "ChatMessage") | select(.message == "Event test message")')
   ```
   **Expected**: 
   - ✅ Chat event found
   - ✅ Contains correct from, message, timestamp

---

## Automated Test Suite

### Unit Tests

Run unit tests:
```bash
cargo test --lib
```

**Expected Coverage**:
- ✅ `crypto.rs`: KeyPair generation, signing, verification
- ✅ `types.rs`: Transaction serialization, block hashing
- ✅ `state.rs`: Transaction validation, balance updates
- ✅ `wallet.rs`: Account management

### Integration Tests

Run integration tests:
```bash
# Start node first
cargo run --bin miraset -- node start &
sleep 2

# Run tests
cargo test --test integration_tests

# Stop node
killall miraset
```

### End-to-End Test

Run complete workflow:
```bash
bash test_demo.sh
```

**Expected**: All operations complete successfully

---

## Performance Benchmarks

### Transaction Throughput

**Test**: Submit 100 transactions and measure time

```bash
#!/bin/bash

START_TIME=$(date +%s)

for i in {1..100}; do
    cargo run --bin miraset -- wallet transfer genesis $BOB_ADDR 1 &
done

wait
sleep 6

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

echo "100 transactions in $DURATION seconds"
echo "TPS: $((100 / DURATION))"
```

**Expected**: 
- ✅ All transactions processed
- ✅ ~20 TPS (limited by 5-second block time)

### RPC Response Time

**Test**: Measure RPC latency

```bash
for i in {1..100}; do
    START=$(date +%s%N)
    curl -s http://127.0.0.1:9944/block/latest > /dev/null
    END=$(date +%s%N)
    DURATION=$(( (END - START) / 1000000 ))
    echo "Request $i: ${DURATION}ms"
done | awk '{sum+=$3; count++} END {print "Average:", sum/count, "ms"}'
```

**Expected**: 
- ✅ Average < 10ms (local)
- ✅ No timeouts
- ✅ Consistent latency

---

## Error Scenarios

### Test Invalid Inputs

1. **Invalid Address Format**
   ```bash
   curl http://127.0.0.1:9944/balance/invalid
   ```
   **Expected**: 400 Bad Request

2. **Insufficient Balance**
   ```bash
   cargo run --bin miraset -- wallet transfer alice $BOB_ADDR 999999999
   ```
   **Expected**: Error message "Insufficient balance"

3. **Invalid Nonce**
   - Manually construct transaction with wrong nonce
   **Expected**: Error "Invalid nonce"

4. **Invalid Signature**
   - Manually construct transaction with wrong signature
   **Expected**: Error "Invalid signature"

5. **Empty Chat Message**
   ```bash
   cargo run --bin miraset -- chat send alice ""
   ```
   **Expected**: Error "Invalid message length"

6. **Message Too Long**
   ```bash
   cargo run --bin miraset -- chat send alice "$(printf 'x%.0s' {1..1001})"
   ```
   **Expected**: Error "Invalid message length"

---

## Test Checklist

### Basic Functionality
- [ ] Node starts successfully
- [ ] Genesis account has correct balance
- [ ] Can create new accounts
- [ ] Can import/export accounts
- [ ] Can transfer tokens
- [ ] Can send chat messages
- [ ] Can query balances via RPC
- [ ] Can query blocks via RPC
- [ ] Can query events via RPC

### Advanced Functionality
- [ ] Nonces increment correctly
- [ ] Transactions are validated
- [ ] Events are emitted for all actions
- [ ] Multiple accounts work simultaneously
- [ ] TUI displays data correctly
- [ ] TUI allows sending transactions
- [ ] Account recovery works

### Error Handling
- [ ] Invalid addresses rejected
- [ ] Insufficient balance detected
- [ ] Invalid nonces rejected
- [ ] Invalid signatures rejected
- [ ] Message length validated
- [ ] Nonexistent blocks return 404
- [ ] Proper HTTP status codes

### Performance
- [ ] Blocks produced every 5 seconds
- [ ] RPC response < 10ms
- [ ] Can handle 20+ TPS
- [ ] Memory usage stable
- [ ] No memory leaks

---

## Continuous Integration

### GitHub Actions Workflow

Create `.github/workflows/test.yml`:

```yaml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - name: Run unit tests
        run: cargo test --lib
      - name: Run integration tests
        run: |
          cargo run --bin miraset -- node start &
          sleep 5
          cargo test --test integration_tests
          killall miraset
      - name: Check formatting
        run: cargo fmt -- --check
      - name: Run clippy
        run: cargo clippy -- -D warnings
```

---

## Conclusion

This comprehensive test suite covers:
- ✅ All user scenarios
- ✅ RPC API functionality
- ✅ Error handling
- ✅ Performance characteristics
- ✅ Integration testing
- ✅ End-to-end workflows

Run these tests regularly to ensure system stability and correctness.

---

## Desktop Wallet Smoke Test

**Goal**: Validate the MIRASET GUI wallet against a running RPC node.

**Steps**:

1. **Start node**
   ```bash
   cargo run --bin miraset -- node start
   ```

2. **Run desktop wallet (dev)**
   ```bash
   cd wallet
   bun install
   bun run tauri:dev
   ```

3. **Create account**
   - Use the UI to create `alice`
   - Verify the account appears with an address

4. **Fund account**
   ```bash
   cargo run --bin miraset -- wallet import genesis 0101010101010101010101010101010101010101010101010101010101010101
   cargo run --bin miraset -- wallet transfer genesis <ALICE_ADDR> 10000
   ```

5. **Refresh balance**
   - Click Refresh in the GUI
   - Expect balance to update to `10000` MIRA

6. **Send transfer**
   - Create `bob` in the GUI
   - Send `1000` MIRA from `alice` to `bob`
   - Refresh and verify balances update

**Expected**:
- ✅ Accounts list updates
- ✅ Balances refresh correctly
- ✅ Transfers submit without errors
