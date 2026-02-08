# 🧪 Test Scripts - Quick Reference

## Available Test Scripts

### 1. `test_demo.sh` - Full Functionality Demo

Comprehensive end-to-end test demonstrating all blockchain features.

**Usage:**
```bash
# Run with existing accounts
./test_demo.sh

# Clean wallet and run fresh
./test_demo.sh --clean
```

**What it does:**
1. ✅ Imports genesis account
2. ✅ Creates test accounts (alice, bob)
3. ✅ Transfers tokens
4. ✅ Sends chat messages
5. ✅ Queries blockchain via RPC
6. ✅ Displays all functionality

**Prerequisites:**
- Node must be running: `cargo run --bin miraset -- node start`

---

### 2. `test_rpc_simple.sh` - RPC API Tests

Quick validation of all RPC endpoints.

**Usage:**
```bash
./test_rpc_simple.sh
```

**Tests:**
- ✅ GET /balance/{address}
- ✅ GET /nonce/{address}
- ✅ GET /block/latest
- ✅ GET /block/{height}
- ✅ GET /events
- ✅ GET /chat/messages
- ✅ Error handling (400, 404)

**Prerequisites:**
- Node must be running

---

## Common Issues & Solutions

### Issue: "Account already exists"

**Solution 1** - Run with clean flag:
```bash
./test_demo.sh --clean
```

**Solution 2** - Manually clean wallet:
```bash
rm ~/.miraset/wallet.json
./test_demo.sh
```

---

### Issue: "Connection refused" or RPC errors

**Problem**: Node is not running

**Solution**: Start the node in a separate terminal:
```bash
cargo run --bin miraset -- node start
```

Then run tests in another terminal:
```bash
./test_demo.sh
```

---

### Issue: Genesis balance is 0

**Problem**: Node was restarted (state is in-memory)

**Solution**: 
1. Stop the current node
2. Start a fresh node:
   ```bash
   cargo run --bin miraset -- node start
   ```
3. The genesis account will have the correct balance

---

### Issue: Scripts not executable

**Solution**: Make them executable:
```bash
chmod +x test_demo.sh test_rpc_simple.sh
```

---

## Step-by-Step Testing Guide

### First Time Setup

1. **Start the node:**
   ```bash
   # Terminal 1
   cargo run --bin miraset -- node start
   ```

2. **Run full demo (clean):**
   ```bash
   # Terminal 2
   ./test_demo.sh --clean
   ```

3. **Run RPC tests:**
   ```bash
   ./test_rpc_simple.sh
   ```

---

### Testing Workflow

When testing after code changes:

1. **Stop old node** (Ctrl+C in terminal 1)

2. **Start fresh node:**
   ```bash
   cargo run --bin miraset -- node start
   ```

3. **Clean and test:**
   ```bash
   ./test_demo.sh --clean
   ```

---

## Manual Testing Commands

If you prefer to run commands manually:

### Setup
```bash
# Start node (Terminal 1)
cargo run --bin miraset -- node start

# Import genesis (Terminal 2)
cargo run --bin miraset -- wallet import genesis 0101010101010101010101010101010101010101010101010101010101010101

# Check balance
cargo run --bin miraset -- wallet balance genesis
```

### Create Accounts
```bash
cargo run --bin miraset -- wallet new alice
cargo run --bin miraset -- wallet new bob
cargo run --bin miraset -- wallet list
```

### Transfer Tokens
```bash
# Get Alice's address
ALICE_ADDR=$(cargo run --bin miraset -- wallet list 2>/dev/null | grep alice | awk '{print $3}')

# Transfer
cargo run --bin miraset -- wallet transfer genesis $ALICE_ADDR 1000

# Wait for block (5-10 seconds)
sleep 6

# Check balance
cargo run --bin miraset -- wallet balance alice
```

### Send Chat
```bash
cargo run --bin miraset -- chat send alice "Hello, blockchain!"
sleep 6
cargo run --bin miraset -- chat list
```

### RPC Queries
```bash
# Balance
curl http://127.0.0.1:9944/balance/4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd

# Latest block
curl http://127.0.0.1:9944/block/latest

# Events
curl "http://127.0.0.1:9944/events?from_height=0&limit=10"

# Chat messages
curl "http://127.0.0.1:9944/chat/messages?limit=10"
```

---

## Expected Output

### Successful test_demo.sh run:
```
=====================================
Miraset Chain Full Functionality Demo
=====================================

Step 1: Import Genesis Account
Genesis account already exists, skipping import

Step 2: Check Genesis Balance
Balance for 'genesis': 1000000000000

Step 3: Create Two Test Accounts
Alice account already exists, skipping creation
Bob account already exists, skipping creation

Step 4: List All Accounts
Accounts:
  genesis -> 4ae7bb72...
  alice -> 82a7c778...
  bob -> 1f3e2d4c...

...

=====================================
Demo Complete!
=====================================
```

### Successful test_rpc_simple.sh run:
```
=== RPC Integration Tests ===

Test 1: GET /balance/{address}
Balance: 1000000000000

Test 2: GET /nonce/{address}
Nonce: 0

Test 3: GET /block/latest
Response:
{"height":5,"timestamp":"2026-02-03T...","prev_hash":[...],...}

...

Test 7: Invalid Address (should be 400)
HTTP Status: 400

Test 8: Nonexistent Block (should be 404)
HTTP Status: 404

=== Tests Complete ===
```

---

## Debugging Tips

### Enable verbose output:
```bash
# For test_demo.sh - remove error suppression
# Edit the script and remove `2>/dev/null` from commands

# For RPC - show full responses
curl -v http://127.0.0.1:9944/block/latest
```

### Check node logs:
```bash
# Run node with logs visible
cargo run --bin miraset -- node start

# Watch for:
# - "Produced block #X" messages
# - Transaction submissions
# - Any errors
```

### Verify wallet state:
```bash
cat ~/.miraset/wallet.json
```

### Check if node is running:
```bash
curl http://127.0.0.1:9944/block/latest
# Should return JSON if node is running
```

---

## Unit Tests

Don't forget to run unit tests:

```bash
# All unit tests
cargo test --lib

# Specific module
cargo test --lib -p miraset-core
cargo test --lib -p miraset-node
cargo test --lib -p miraset-wallet

# With output
cargo test --lib -- --nocapture

# Specific test
cargo test test_submit_transfer_valid
```

---

## Integration Tests

Run integration tests (requires running node):

```bash
# Start node
cargo run --bin miraset -- node start &

# Run tests
cargo test --test integration_tests

# Stop node
killall miraset
```

---

## Quick Reference Card

| Command | Purpose |
|---------|---------|
| `./test_demo.sh` | Run full demo |
| `./test_demo.sh --clean` | Clean wallet & run demo |
| `./test_rpc_simple.sh` | Test RPC endpoints |
| `cargo test --lib` | Run all unit tests |
| `cargo run --bin miraset -- node start` | Start node |
| `cargo run --bin miraset-tui` | Launch TUI |
| `rm ~/.miraset/wallet.json` | Clear wallet |

---

## Getting Help

- **Documentation**: Check `TESTING.md` for detailed scenarios
- **Test Report**: See `TEST_REPORT.md` for coverage details
- **User Guide**: Read `USER_GUIDE.md` for complete instructions
- **Issues**: Check node logs for errors

---

**Happy Testing! 🚀**
