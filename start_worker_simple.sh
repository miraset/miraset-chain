#!/bin/bash
# Simple Worker Startup Script (No jq required)

echo "🚀 Starting Miraset Worker"
echo "=========================="
echo ""

# Step 1: Check node
echo "1️⃣  Checking node..."
if curl -s http://127.0.0.1:9944/block/latest > /dev/null 2>&1; then
    echo "   ✅ Node is running on port 9944"
else
    echo "   ❌ Node not running!"
    echo ""
    echo "   Please start node first:"
    echo "   Terminal 1: cargo run --bin miraset -- node start"
    echo ""
    exit 1
fi

# Step 2: Check if worker already running
echo ""
echo "2️⃣  Checking port 8080..."
if curl -s http://127.0.0.1:8080/health > /dev/null 2>&1; then
    echo "   ⚠️  Worker already running on port 8080"
    echo "   Kill it first: pkill -f miraset-worker"
    exit 1
else
    echo "   ✅ Port 8080 is available"
fi

# Step 3: Start worker
echo ""
echo "3️⃣  Starting worker..."
echo "   Building and starting miraset-worker..."
echo ""
echo "════════════════════════════════════════"
echo ""

cargo run --bin miraset-worker

# This line only reached if worker exits
echo ""
echo "Worker stopped."
