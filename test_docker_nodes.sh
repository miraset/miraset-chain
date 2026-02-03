#!/bin/bash
# Test multi-node Docker setup
echo "Testing 4-node setup..."
for port in 9944 9945 9946 9947; do
    echo -n "Node on port $port: "
    if curl -s http://localhost:$port/block/latest > /dev/null 2>&1; then
        echo "✅ RUNNING"
    else
        echo "❌ NOT RESPONDING"
    fi
done
echo ""
echo "Run 'docker-compose logs -f' to view logs"
