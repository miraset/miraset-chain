#!/bin/bash
# One-command test script

echo "🚀 Miraset Quick Test"
echo "===================="
echo ""
echo "Starting services and running test..."
echo ""

# Check if node is running
if ! curl -s http://127.0.0.1:9944/block/latest > /dev/null 2>&1; then
    echo "❌ Start node first: cargo run --bin miraset -- node start"
    exit 1
fi

# Check if worker is running
if ! curl -s http://127.0.0.1:8080/health > /dev/null 2>&1; then
    echo "❌ Start worker first: cargo run --bin miraset-worker"
    exit 1
fi

echo "✅ Services are running"
echo ""

# Run E2E test
./test_worker_e2e.sh

echo ""
echo "📋 For more details see: FINAL_STATUS.md"
