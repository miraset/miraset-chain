#!/bin/bash

# Docker Build & Test Script

set -e

echo "======================================"
echo "  Docker Build & Test"
echo "======================================"
echo ""

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}Step 1: Check Docker${NC}"
if ! command -v docker &> /dev/null; then
    echo -e "${RED}Docker not found!${NC}"
    exit 1
fi
echo -e "${GREEN}✅ Docker found${NC}"
docker --version
echo ""

echo -e "${BLUE}Step 2: Build Docker image${NC}"
echo "This may take 5-10 minutes on first build..."
docker build -t miraset-chain:test . || {
    echo -e "${RED}❌ Build failed!${NC}"
    exit 1
}
echo -e "${GREEN}✅ Build successful${NC}"
echo ""

echo -e "${BLUE}Step 3: Start test container${NC}"
docker-compose -f docker-compose.test.yml up -d
sleep 5
echo -e "${GREEN}✅ Container started${NC}"
echo ""

echo -e "${BLUE}Step 4: Check if node is running${NC}"
sleep 3
if curl -s http://localhost:9944/block/latest > /dev/null 2>&1; then
    echo -e "${GREEN}✅ Node is responding!${NC}"
    echo ""
    echo "Latest block:"
    curl -s http://localhost:9944/block/latest | head -c 200
    echo ""
else
    echo -e "${RED}❌ Node not responding${NC}"
    echo ""
    echo "Container logs:"
    docker-compose -f docker-compose.test.yml logs
fi
echo ""

echo -e "${BLUE}Step 5: Cleanup${NC}"
echo "Stop container? (y/n)"
read -r answer
if [ "$answer" = "y" ]; then
    docker-compose -f docker-compose.test.yml down
    echo -e "${GREEN}✅ Cleaned up${NC}"
else
    echo "Container still running. Use: docker-compose -f docker-compose.test.yml down"
fi

echo ""
echo "======================================"
echo "  Test Complete"
echo "======================================"
