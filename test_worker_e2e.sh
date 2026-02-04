#!/bin/bash
# End-to-end test: Node + Worker integration
set -e
echo "🔧 Miraset Worker E2E Test"
echo "=========================="
echo ""
# Check if node is running
if ! curl -s http://127.0.0.1:9944/block/latest > /dev/null 2>&1; then
    echo "❌ Node not running on port 9944"
    echo "   Start node first: cargo run --bin miraset -- node start"
    exit 1
fi
echo "✓ Node is running"
# Check if worker is running
if ! curl -s http://127.0.0.1:8080/health > /dev/null 2>&1; then
    echo "❌ Worker not running on port 8080"
    echo "   Start worker: cargo run --bin miraset-worker"
    exit 1
fi
echo "✓ Worker is running"
echo ""
# Test job flow
JOB_ID="0000000000000000000000000000000000000000000000000000000000000042"
echo "1️⃣  Accepting job..."
curl -s -X POST http://127.0.0.1:8080/jobs/accept \
  -H "Content-Type: application/json" \
  -d "{
    \"job_id\": \"$JOB_ID\",
    \"epoch_id\": 1,
    \"model_id\": \"gemma3:latest\",
    \"max_tokens\": 50,
    \"price_per_token\": 10
  }"
echo ""
echo ""
echo "2️⃣  Running job..."
curl -s -X POST http://127.0.0.1:8080/jobs/run \
  -H "Content-Type: application/json" \
  -d "{
    \"job_id\": \"$JOB_ID\",
    \"prompt\": \"What is blockchain?\",
    \"temperature\": 0.7
  }"
echo ""
echo ""
echo "3️⃣  Checking job status..."
curl -s http://127.0.0.1:8080/jobs/$JOB_ID/status
echo ""
echo ""
echo "4️⃣  Generating receipt..."
curl -s -X POST http://127.0.0.1:8080/jobs/$JOB_ID/report
echo ""
echo ""
echo "5️⃣  Checking chain events..."
curl -s http://127.0.0.1:9944/events?limit=5 | head -20
echo ""
echo ""
echo "✅ End-to-end test completed!"
echo ""
echo "Next steps:"
echo "  - Check worker registration: curl http://127.0.0.1:9944/events"
echo "  - Check job completion: curl http://127.0.0.1:9944/events"
