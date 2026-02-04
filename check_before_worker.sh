#!/bin/bash
# Quick checklist before running worker

echo "📋 Pre-flight Checklist"
echo "======================"
echo ""

# Check 1: Build status
echo "1️⃣  Checking build..."
if cargo build --bin miraset-worker --quiet 2>&1 | grep -q "error"; then
    echo "   ❌ Build failed! Run: cargo build --bin miraset-worker"
    exit 1
else
    echo "   ✅ Worker builds successfully"
fi

# Check 2: Node status
echo ""
echo "2️⃣  Checking if node is running..."
if curl -s http://127.0.0.1:9944/block/latest > /dev/null 2>&1; then
    echo "   ✅ Node is running on port 9944"
    LATEST_BLOCK=$(curl -s http://127.0.0.1:9944/block/latest | grep -oP '"height":\K[0-9]+' 2>/dev/null || echo "unknown")
    echo "   📊 Latest block height: $LATEST_BLOCK"
else
    echo "   ❌ Node is NOT running!"
    echo ""
    echo "   ⚠️  You MUST start the node first:"
    echo "      Terminal 1: cargo run --bin miraset -- node start"
    echo ""
    echo "   Wait for: 'RPC server listening on 127.0.0.1:9944'"
    echo ""
    exit 1
fi

# Check 3: Port 8080 availability
echo ""
echo "3️⃣  Checking port 8080..."
if curl -s http://127.0.0.1:8080/health > /dev/null 2>&1; then
    echo "   ⚠️  Port 8080 already in use (worker may be running)"
    echo "   Stop it first: pkill -f miraset-worker"
else
    echo "   ✅ Port 8080 is available"
fi

# Check 4: Ollama (optional)
echo ""
echo "4️⃣  Checking Ollama (optional)..."
if curl -s http://localhost:11434/api/version > /dev/null 2>&1; then
    echo "   ✅ Ollama is running"
    VERSION=$(curl -s http://localhost:11434/api/version | jq -r '.version' 2>/dev/null || echo "unknown")
    echo "   📦 Version: $VERSION"
else
    echo "   ⚠️  Ollama not running (will use mock inference)"
    echo "   To install: https://ollama.ai"
fi

echo ""
echo "════════════════════════════════"
echo "✅ All checks passed!"
echo ""
echo "🚀 Ready to start worker:"
echo "   cargo run --bin miraset-worker"
echo ""
echo "📝 Or use test script:"
echo "   ./test_worker_e2e.sh"
echo ""
