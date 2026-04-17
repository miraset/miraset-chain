# Worker Integration Guide

## ⚠️ IMPORTANT: Start Order

**You MUST start the node first, then the worker!**

## Port Configuration

- **Node RPC**: `127.0.0.1:9944` (default)
- **Worker API**: `127.0.0.1:8080`
- **Ollama**: `localhost:11434` (optional)

## Quick Start (Step-by-Step)

### Step 0: Pre-flight Check (Recommended)
Run the checklist to verify everything is ready:
```bash
./check_before_worker.sh
```

This will check:
- ✅ Worker builds successfully
- ✅ Node is running on port 9944
- ✅ Port 8080 is available
- ✅ Ollama status (optional)

### Step 1: Start Node (Terminal 1)
```bash
cargo run --bin miraset -- node start
```

**Wait for this message:**
```
RPC server listening on 127.0.0.1:9944
```

**Verify node is running:**
```bash
# In another terminal
curl http://127.0.0.1:9944/health
```

If you see JSON with status data, node is ready! ✅

### Step 2: Start Worker (Terminal 2)
**Only after node is running!**

```bash
cargo run --bin miraset-worker
```

**Expected output:**
```
✓ Worker registered on-chain with ID: d50a2c3a9f074ded...
Worker listening on 127.0.0.1:8080
Connecting to node at http://127.0.0.1:9944
```

**Verify worker is running:**
```bash
# In another terminal
curl http://127.0.0.1:8080/health
```

Should return: `{"status":"healthy","timestamp":"..."}`

### Step 3: Test E2E (Terminal 3)
```bash
./test_worker_e2e.sh
```

## Common Errors & Solutions

### ❌ Error: "No connection could be made" (port 9944)
```
error trying to connect: tcp connect error: No connection could be made
because the target machine actively refused it. (os error 10061)
```

**Cause**: Node is not running
**Solution**: 
1. Start node FIRST in Terminal 1
2. Wait 3-5 seconds for "RPC server listening on 127.0.0.1:9944"
3. THEN start worker

### ❌ Error: "Path segments must not start with `:` "
```
Path segments must not start with `:`. For capture groups, use `{capture}`.
```

**Cause**: Old code (already fixed)
**Solution**: Rebuild worker
```bash
cargo build --bin miraset-worker
```

### ✅ Success Indicators

**Node is ready when you see:**
```
RPC server listening on 127.0.0.1:9944
Blockchain initialized with genesis block
```

**Worker is ready when you see:**
```
✓ Worker registered on-chain with ID: ...
Worker listening on 127.0.0.1:8080
```

## Complete Flow

1. **Worker Registration**: Auto-registers on-chain with GPU specs
2. **Job Assignment**: Accept via `POST /jobs/accept`
3. **Job Execution**: Run via `POST /jobs/run`
4. **Auto-Submit**: Worker submits result + receipt to chain
5. **Verification**: Receipt hash anchored on-chain

## API Examples

### Health Check
```bash
# Node
curl http://localhost:9944/health
curl http://localhost:9944/status
curl http://localhost:9944/ping

# Worker
curl http://localhost:8080/health
curl http://localhost:8080/status
curl http://localhost:8080/ping
```

### Accept Job
```bash
curl -X POST http://localhost:8080/jobs/accept \
  -H "Content-Type: application/json" \
  -d '{
    "job_id":"0000000000000000000000000000000000000000000000000000000000000042",
    "epoch_id":1,
    "model_id":"llama2",
    "max_tokens":100,
    "price_per_token":10
  }'
```

### Run Job
```bash
curl -X POST http://localhost:8080/jobs/run \
  -H "Content-Type: application/json" \
  -d '{
    "job_id":"0000000000000000000000000000000000000000000000000000000000000042",
    "prompt":"Explain quantum computing",
    "temperature":0.7
  }'
```

### Check Status
```bash
curl http://localhost:8080/jobs/0000000000000000000000000000000000000000000000000000000000000042/status
```

## Architecture

```
Node (9944) ◄──RPC──► Worker (8080) ◄──HTTP──► Ollama (11434)
     │                      │
     │  RegisterWorker      │
     │◄─────────────────────┤
     │                      │
     │  SubmitJobResult     │
     │◄─────────────────────┤
```

## Files Created

- `crates/miraset-worker/src/node_client.rs` - RPC client for node
- `crates/miraset-worker/README.md` - Worker documentation
- `run_worker.sh` - Worker startup script
- `test_worker_e2e.sh` - End-to-end test
- `start_demo.sh` - Start both services

## Monitoring Events

```bash
# Check worker registration
curl http://127.0.0.1:9944/events | jq '.[] | select(.type=="WorkerRegistered")'

# Check job results
curl http://127.0.0.1:9944/events | jq '.[] | select(.type=="JobCompleted")'

# All recent events
curl http://127.0.0.1:9944/events?limit=10
```

## Automated Start (Alternative)

Use the demo script to start both services:
```bash
./start_demo.sh
# Starts node and worker in background
# Check logs: tail -f node.log worker.log
```

## Stopping Services

```bash
# Find processes
ps aux | grep miraset

# Kill by process name
pkill -f "miraset.*node"
pkill -f "miraset-worker"

# Or use Ctrl+C in each terminal
```

## Need Help?

Check logs for detailed error messages:
- Node logs: Terminal 1 output
- Worker logs: Terminal 2 output
- Or: `tail -f node.log worker.log` (if using start_demo.sh)
