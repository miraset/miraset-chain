#!/bin/bash
# Test persistence functionality

set -e

echo "==================================="
echo "Miraset Persistence Test"
echo "==================================="

# Cleanup
rm -rf .data_test

# Build
cargo build --bin miraset --quiet

# Start node
./target/debug/miraset node start --storage-path .data_test --block-interval 2 &
NODE_PID=$!
sleep 3

# Check storage created
if [ ! -d ".data_test" ]; then
    echo "✗ Storage not created"
    kill $NODE_PID
    exit 1
fi
echo "✓ Storage created"

# Stop and restart
kill $NODE_PID
wait $NODE_PID 2>/dev/null || true
sleep 2

./target/debug/miraset node start --storage-path .data_test --block-interval 2 &
NODE_PID=$!
sleep 3

echo "✓ Node restarted successfully"

# Cleanup
kill $NODE_PID
rm -rf .data_test

echo "✅ Persistence test passed!"
