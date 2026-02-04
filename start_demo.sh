
#!/bin/bash
# Quick Demo: Start Node + Worker

echo "🚀 Miraset Quick Demo"
echo "===================="
echo ""

# Check if Windows (Git Bash)
if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
    CMD_PREFIX="cmd.exe /c"
else
    CMD_PREFIX=""
fi

# Start node in background
echo "1️⃣  Starting Node..."
cargo run --bin miraset -- node start > node.log 2>&1 &
NODE_PID=$!
echo "   Node PID: $NODE_PID"

# Wait for node to start
echo "   Waiting for node to be ready..."
sleep 3

# Check if node is responding
if curl -s http://127.0.0.1:9944/block/latest > /dev/null 2>&1; then
    echo "   ✓ Node is ready!"
else
    echo "   ⚠️  Node may still be starting..."
fi

echo ""
echo "2️⃣  Starting Worker..."
cargo run --bin miraset-worker > worker.log 2>&1 &
WORKER_PID=$!
echo "   Worker PID: $WORKER_PID"

# Wait for worker to start
echo "   Waiting for worker to be ready..."
sleep 3

# Check if worker is responding
if curl -s http://127.0.0.1:8080/health > /dev/null 2>&1; then
    echo "   ✓ Worker is ready!"
else
    echo "   ⚠️  Worker may still be starting..."
fi

echo ""
echo "✅ Services Started"
echo ""
echo "📋 Status:"
echo "   Node:   http://127.0.0.1:9944 (PID: $NODE_PID)"
echo "   Worker: http://127.0.0.1:8080 (PID: $WORKER_PID)"
echo ""
echo "📝 Logs:"
echo "   tail -f node.log"
echo "   tail -f worker.log"
echo ""
echo "🧪 Test:"
echo "   ./test_worker_e2e.sh"
echo ""
echo "⏹️  Stop:"
echo "   kill $NODE_PID $WORKER_PID"
echo ""
