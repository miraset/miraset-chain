# Port Configuration Fix - Summary

## ✅ FIXED: Port Mismatch Issue

### Problem
Worker was trying to connect to port **3000**, but node runs on port **9944**.

### Root Cause
- Node default RPC port: `127.0.0.1:9944` (defined in `crates/miraset-cli/src/main.rs`)
- Worker was incorrectly configured to use: `127.0.0.1:3000`

### Solution
Updated all files to use correct port **9944**.

## Files Updated

### 1. Worker Binary Configuration
**File**: `crates/miraset-worker/src/main.rs`
```rust
node_url: "http://127.0.0.1:9944".to_string(),  // ✅ Corrected
```

### 2. Documentation
**Files**: 
- `WORKER_INTEGRATION.md` - All port references updated
- `check_before_worker.sh` - Node check uses port 9944
- `test_worker_e2e.sh` - All curl commands use port 9944
- `start_demo.sh` - Port references updated
- `run_worker.sh` - Node check uses port 9944

### 3. Tests
**File**: `crates/miraset-worker/src/lib.rs`
```rust
node_url: "http://127.0.0.1:9944".to_string(),  // ✅ Test configs updated
```

## Correct Port Configuration

```
┌──────────────────────────────────────┐
│         Port Assignment              │
├──────────────────────────────────────┤
│ Node RPC:      127.0.0.1:9944       │
│ Worker API:    127.0.0.1:8080       │
│ Ollama:        127.0.0.1:11434      │
└──────────────────────────────────────┘
```

## How to Verify

### 1. Check Node is Running
```bash
curl http://127.0.0.1:9944/block/latest
```

### 2. Run Pre-flight Check
```bash
./check_before_worker.sh
```

Expected output:
```
2️⃣  Checking if node is running...
   ✅ Node is running on port 9944
   📊 Latest block height: 0
```

### 3. Start Worker
```bash
cargo run --bin miraset-worker
```

Expected output:
```
✓ Worker registered on-chain with ID: ...
Worker listening on 127.0.0.1:8080
Connecting to node at http://127.0.0.1:9944  ✅
```

## Testing

### Run E2E Test
```bash
./test_worker_e2e.sh
```

This now correctly checks:
- Node on port 9944 ✅
- Worker on port 8080 ✅

### Manual Test
```bash
# 1. Check node
curl http://127.0.0.1:9944/block/latest

# 2. Check worker
curl http://127.0.0.1:8080/health

# 3. Check events
curl http://127.0.0.1:9944/events?limit=5
```

## Configuration Override (if needed)

To use a different port for node, set in `miraset.toml`:
```toml
[node]
rpc_addr = "127.0.0.1:3000"  # Custom port
```

Or use environment variable:
```bash
export MIRASET_RPC_ADDR="127.0.0.1:3000"
cargo run --bin miraset -- node start
```

Then update worker config:
```rust
node_url: "http://127.0.0.1:3000".to_string(),
```

## Status: ✅ RESOLVED

All port references have been corrected to use **9944** for node RPC.

Worker will now successfully connect to node on startup!

### ⚠️ Note: jq is NOT required
All test scripts have been updated to work without `jq`. If you see "bash: jq: command not found", that's fine - the scripts will still work!

## Quick Start Commands

```bash
# Terminal 1: Start Node
cargo run --bin miraset -- node start
# Wait for: "RPC server listening on 127.0.0.1:9944"

# Terminal 2: Verify & Start Worker
./check_before_worker.sh  # Verify node is ready (no jq needed)
cargo run --bin miraset-worker

# OR use simple startup script:
./start_worker_simple.sh

# Terminal 3: Test
./test_worker_e2e.sh  # Works without jq!
```

## Next Steps

1. ✅ Start node on port 9944
2. ✅ Run check script to verify
3. ✅ Start worker (will connect to 9944)
4. ✅ Run E2E tests

Everything should now work correctly! 🎉
