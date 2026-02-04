#!/bin/bash
# Run Miraset Worker

set -e

echo "🔧 Starting Miraset Worker..."
echo ""

# Check if node is running
if ! curl -s http://127.0.0.1:9944/block/latest > /dev/null 2>&1; then
    echo "⚠️  Warning: Node not detected on port 9944"
    echo "   Start node first: cargo run --bin miraset -- node start"
    echo ""
fi

# Check if Ollama is running
if ! curl -s http://localhost:11434/api/version > /dev/null 2>&1; then
    echo "⚠️  Warning: Ollama not detected on port 11434"
    echo "   Worker will use mock inference"
    echo ""
fi

echo "🚀 Starting worker on http://127.0.0.1:8080"
echo ""

cargo run --bin miraset-worker
