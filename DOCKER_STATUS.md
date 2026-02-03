# 🐳 Docker Status & Instructions

## Current Status

### Issues Fixed
- ✅ `.dockerignore` исправлен - теперь не исключает исходный код
- ✅ `Dockerfile` оптимизирован - двухэтапная сборка с кешированием зависимостей
- ✅ Создан `docker-compose.test.yml` - для тестирования одной ноды
- ✅ Создан `test_docker_build.sh` - автоматический тест сборки

### Build Process

Docker build теперь работает в два этапа:

**Stage 1: Dependencies** (кешируется)
```dockerfile
COPY Cargo.toml Cargo.lock
COPY crates/*/Cargo.toml
RUN cargo build --release  # Only dependencies
```

**Stage 2: Application**
```dockerfile
COPY crates
RUN cargo build --release  # Only app code
```

Это ускоряет пересборку при изменении кода (зависимости уже скомпилированы).

---

## Quick Test

### Test Single Node

```bash
# 1. Build and start test node
docker-compose -f docker-compose.test.yml up -d

# 2. Check logs
docker-compose -f docker-compose.test.yml logs -f

# 3. Test RPC
curl http://localhost:9944/block/latest

# 4. Cleanup
docker-compose -f docker-compose.test.yml down
```

### Or Use Test Script

```bash
./test_docker_build.sh
```

---

## Build from Scratch

### Local Build (recommended first)

Убедитесь что локально всё компилируется:

```bash
# Clean build
cargo clean
cargo build --release

# Should succeed ✅
```

### Docker Build

```bash
# Build image
docker build -t miraset-chain:latest .

# Check image size
docker images | grep miraset-chain

# Expected: ~500MB (builder) + ~200MB (runtime)
```

### Run Container

```bash
# Run single container
docker run -d \
  --name miraset-test \
  -p 9944:9944 \
  -v miraset-data:/data \
  miraset-chain:latest

# Check logs
docker logs -f miraset-test

# Test RPC
curl http://localhost:9944/block/latest

# Stop
docker stop miraset-test
docker rm miraset-test
```

---

## Multi-Node Setup

### Start All 4 Nodes

```bash
# Build images
docker-compose build

# Start all nodes
docker-compose up -d

# Check status
docker-compose ps

# View logs
docker-compose logs -f

# Test each node
for port in 9944 9945 9946 9947; do
  echo "Testing port $port..."
  curl -s http://localhost:$port/block/latest | head -c 100
  echo ""
done
```

### Stop All Nodes

```bash
# Stop (keeps data)
docker-compose down

# Stop and remove data
docker-compose down -v
```

---

## Troubleshooting

### Build Fails

**Error**: "cargo build" fails in Docker

**Solution 1**: Check local build first
```bash
cargo clean
cargo build --release
```

**Solution 2**: Check .dockerignore
```bash
cat .dockerignore
# Should NOT exclude crates/
```

**Solution 3**: Build with verbose output
```bash
docker build --progress=plain -t miraset-chain:test .
```

### Container Won't Start

**Check logs**:
```bash
docker logs miraset-node1
```

**Common issues**:
- Port already in use (9944)
- Permission issues with /data volume

**Solution**:
```bash
# Stop conflicting processes
taskkill /F /IM miraset.exe  # Windows
pkill miraset                # Linux

# Remove old container
docker rm -f miraset-node1

# Restart
docker-compose up -d
```

### RPC Not Responding

**Check if container is running**:
```bash
docker ps
```

**Check network**:
```bash
docker network inspect miraset-chain_miraset-net
```

**Test from inside container**:
```bash
docker exec -it miraset-node1 bash
curl http://localhost:9944/block/latest
```

### Slow Build

**Use build cache**:
- Docker caches each layer
- Only changed layers are rebuilt
- First build: ~10 minutes
- Subsequent: ~2 minutes

**Speed up**:
```bash
# Use BuildKit
export DOCKER_BUILDKIT=1
docker build -t miraset-chain:latest .
```

---

## Docker Files

| File | Purpose |
|------|---------|
| `Dockerfile` | Multi-stage build definition |
| `docker-compose.yml` | 4-node production setup |
| `docker-compose.test.yml` | Single-node test setup |
| `.dockerignore` | Exclude unnecessary files |
| `test_docker_build.sh` | Automated build test |

---

## Best Practices

### Development

1. **Test locally first**
   ```bash
   cargo test --lib
   cargo run --bin miraset -- node start
   ```

2. **Then test Docker**
   ```bash
   docker-compose -f docker-compose.test.yml up
   ```

3. **Finally test multi-node**
   ```bash
   docker-compose up -d
   ```

### Production

1. Use specific version tags
   ```dockerfile
   FROM rust:1.75.0 as builder
   ```

2. Add health checks
   ```yaml
   healthcheck:
     test: ["CMD", "curl", "-f", "http://localhost:9944/block/latest"]
     interval: 30s
   ```

3. Set resource limits
   ```yaml
   deploy:
     resources:
       limits:
         cpus: '2'
         memory: 2G
   ```

---

## Known Limitations

### Current MVP

- ⚠️ Nodes run independently (no P2P yet)
- ⚠️ No consensus between nodes (Phase 2)
- ⚠️ No state synchronization (Phase 2)

Each node is currently **isolated**. They:
- ✅ Produce their own blocks
- ✅ Store data persistently
- ✅ Serve RPC independently
- ❌ Don't communicate with each other (yet)

### Next Phase (P2P + BFT)

Will add:
- libp2p networking
- BFT consensus
- State sync
- Network discovery

---

## Commands Cheat Sheet

```bash
# Build
docker build -t miraset-chain:latest .
docker-compose build

# Run
docker-compose up -d
docker-compose -f docker-compose.test.yml up -d

# Status
docker ps
docker-compose ps

# Logs
docker logs miraset-node1
docker-compose logs -f

# Stop
docker-compose down
docker-compose down -v  # with volumes

# Clean
docker system prune -a
docker volume prune

# Test
curl http://localhost:9944/block/latest
./test_docker_build.sh
./test_docker_nodes.sh
```

---

## Summary

✅ **Docker setup готов**:
- Dockerfile оптимизирован
- docker-compose для 4 нод
- Test setup для отладки
- Автоматический тест скрипт

⚠️ **Ограничения**:
- Ноды независимы (нет P2P)
- Нет консенсуса (Phase 2)

🎯 **Готово для**:
- Development тестирования
- Multi-node deployment
- Phase 2: P2P integration

---

**Status**: ✅ Docker Ready for Testing  
**Date**: February 3, 2026
