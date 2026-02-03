# 💾 Persistence & Multi-Node Setup

## Overview

Miraset Chain now supports:
1. **Persistent Storage** - Data survives node restarts (RocksDB)
2. **Docker Multi-Node** - Run 4 nodes for BFT consensus testing

---

## 🗄️ Persistent Storage

### How It Works

- **Engine**: Sled (pure Rust embedded database)
- **Location**: `.data` directory (configurable via CLI or config file)
- **Data Stored**:
  - Blocks (all blockchain history)
  - Account balances
  - Account nonces
  - Events
  - Latest block pointer

### Features

✅ **Automatic**: Data persists across restarts  
✅ **Fast**: Sled optimized for modern hardware  
✅ **Pure Rust**: No C dependencies, works on Windows  
✅ **Reliable**: ACID guarantees  
✅ **Cross-platform**: Same code on all OSes  
✅ **Configurable**: Set storage path via CLI, env, or config file

### Configuration

Storage path can be configured via:

1. **CLI flags** (highest priority):
   ```bash
   cargo run --bin miraset -- node start --storage-path /custom/path
   ```

2. **Environment variables**:
   ```bash
   export MIRASET_STORAGE_PATH=/custom/path
   cargo run --bin miraset -- node start
   ```

3. **Config file** (`miraset.toml` in project root):
   ```toml
   [node]
   storage_path = ".data"
   rpc_addr = "127.0.0.1:9944"
   block_interval = 5
   ```

4. **Default**: `.data` (relative to working directory)  

### Usage

Storage is **automatic** - no configuration needed!

```bash
# Start node (creates .data if doesn't exist)
cargo run --bin miraset -- node start

# Data persists here
ls .data/

# Restart node - data is still there!
cargo run --bin miraset -- node start

# Custom storage path
cargo run --bin miraset -- node start --storage-path /my/data

# Custom block interval
cargo run --bin miraset -- node start --block-interval 10
```

### Storage Module API

```rust
use miraset_node::Storage;

// Open storage
let storage = Storage::open(".data")?;

// Save block
storage.save_block(&block)?;

// Load block
let block = storage.get_block(height)?;

// Save balance
storage.save_balance(&address, 1000)?;

// Get balance
let balance = storage.get_balance(&address)?;
```

### Data Management

```bash
# Backup data
cp -r .data .data.backup

# Clear all data (fresh start)
rm -rf .data

# View storage size
du -sh .data
```

---

## 🐳 Docker Multi-Node Setup

### Prerequisites

- Docker installed
- Docker Compose installed
- At least 4GB RAM
- Ports 9944-9947 available

### Quick Start

```bash
# 1. Build images (first time only)
docker-compose build

# 2. Start 4 nodes
docker-compose up -d

# 3. Check status
docker ps

# 4. Test nodes
./test_docker_nodes.sh

# 5. View logs
docker-compose logs -f

# 6. Stop nodes
docker-compose down
```

### Architecture

```
┌─────────────────────────────────────────────┐
│            Host Machine                      │
│                                              │
│  ┌──────────┐  ┌──────────┐                │
│  │   CLI    │  │ Your App │                │
│  └────┬─────┘  └────┬─────┘                │
│       │             │                       │
├───────┼─────────────┼───────────────────────┤
│       │             │                       │
│   Port 9944     Port 9945-9947             │
│       │             │                       │
├───────┼─────────────┼───────────────────────┤
│   Docker Network (172.20.0.0/16)           │
│                                              │
│  ┌──────┐  ┌──────┐  ┌──────┐  ┌──────┐   │
│  │Node1 │  │Node2 │  │Node3 │  │Node4 │   │
│  │ :9944│  │ :9944│  │ :9944│  │ :9944│   │
│  │.11   │  │.12   │  │.13   │  │.14   │   │
│  └──┬───┘  └──┬───┘  └──┬───┘  └──┬───┘   │
│     │         │         │         │        │
│  ┌──▼─────────▼─────────▼─────────▼────┐   │
│  │     Persistent Volumes (RocksDB)    │   │
│  └─────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

### Node Configuration

| Node | Container Port | Host Port | IP Address | Volume |
|------|---------------|-----------|------------|--------|
| node1 | 9944 | 9944 | 172.20.0.11 | node1-data |
| node2 | 9944 | 9945 | 172.20.0.12 | node2-data |
| node3 | 9944 | 9946 | 172.20.0.13 | node3-data |
| node4 | 9944 | 9947 | 172.20.0.14 | node4-data |

### Testing Multi-Node

```bash
# Test all nodes
./test_docker_nodes.sh

# Or manually
curl http://localhost:9944/block/latest  # node1
curl http://localhost:9945/block/latest  # node2
curl http://localhost:9946/block/latest  # node3
curl http://localhost:9947/block/latest  # node4
```

### Common Commands

```bash
# View logs
docker-compose logs -f
docker-compose logs node1

# Restart specific node
docker-compose restart node2

# Stop all
docker-compose down

# Remove volumes (DESTRUCTIVE!)
docker-compose down -v

# Build without cache
docker-compose build --no-cache

# Scale nodes (not yet supported)
# docker-compose up -d --scale node=6
```

---

## 🔄 Data Persistence in Docker

### Volumes

Each node has its own persistent volume:

```bash
# List volumes
docker volume ls | grep miraset

# Inspect volume
docker volume inspect miraset-chain_node1-data

# Backup volume
docker run --rm \
  -v miraset-chain_node1-data:/data \
  -v $(pwd):/backup \
  ubuntu tar czf /backup/node1.tar.gz /data

# Restore volume
docker run --rm \
  -v miraset-chain_node1-data:/data \
  -v $(pwd):/backup \
  ubuntu tar xzf /backup/node1.tar.gz -C /
```

### Data Location

**On Host** (when running locally):
```
.data/
```

**In Docker** (mapped to volume):
```
/data/ → docker volume (managed by Docker)
```

---

## 🧪 Testing Scenarios

### Scenario 1: Node Restart Persistence

```bash
# Start nodes
docker-compose up -d

# Submit transaction
cargo run --bin miraset -- wallet transfer genesis <addr> 1000

# Wait for block
sleep 6

# Check balance
curl http://localhost:9944/balance/<addr>

# Restart node
docker-compose restart node1

# Check balance again - should persist!
curl http://localhost:9944/balance/<addr>
```

### Scenario 2: Multi-Node Sync (Future)

Currently nodes run independently. Future BFT consensus will sync state.

```bash
# Submit to node1
curl -X POST http://localhost:9944/tx/submit -d '{...}'

# Check on node2 (will see after consensus)
curl http://localhost:9945/block/latest
```

### Scenario 3: Node Failure Recovery

```bash
# Stop node2
docker-compose stop node2

# Continue using node1
curl http://localhost:9944/block/latest

# Restart node2 - catches up
docker-compose start node2
```

---

## 📈 Monitoring

### Resource Usage

```bash
# CPU, Memory, Network usage
docker stats

# Specific node
docker stats miraset-node1
```

### Logs

```bash
# All logs
docker-compose logs

# Follow logs
docker-compose logs -f

# Last 100 lines
docker-compose logs --tail=100

# Specific node with timestamps
docker-compose logs -t node1
```

### Health Checks

```bash
# Check if responsive
for port in 9944 9945 9946 9947; do
  curl -s http://localhost:$port/block/latest > /dev/null && echo "Port $port: OK" || echo "Port $port: FAIL"
done
```

---

## 🚀 Production Deployment

### Before Production

⚠️ **DO NOT use in production yet!**

The following must be implemented:
- [ ] P2P networking
- [ ] BFT consensus
- [ ] State synchronization
- [ ] Security hardening
- [ ] Load testing
- [ ] Monitoring/alerting
- [ ] Backup automation

### Security Checklist

- [ ] Change default genesis key
- [ ] Enable TLS for RPC
- [ ] Firewall configuration
- [ ] Resource limits
- [ ] Regular backups
- [ ] Log monitoring
- [ ] Intrusion detection

---

## 🔧 Development Tips

### Local Development

```bash
# Run single node locally (with persistence)
cargo run --bin miraset -- node start

# Data in ./data
ls -lh ./data

# Run tests
cargo test --lib
```

### Docker Development

```bash
# Build and test
docker-compose build
docker-compose up -d
./test_docker_nodes.sh

# Make changes
vim crates/miraset-node/src/state.rs

# Rebuild and restart
docker-compose build node1
docker-compose restart node1

# Check logs
docker-compose logs -f node1
```

### Debugging

```bash
# Enter container
docker exec -it miraset-node1 bash

# Inside container
ls -lh /data
cat /data/LOG
```

---

## 📚 References

- [DOCKER.md](DOCKER.md) - Detailed Docker documentation
- [Storage Module](crates/miraset-node/src/storage.rs) - Storage implementation
- [docker-compose.yml](docker-compose.yml) - Multi-node configuration

---

## ✅ Status

**Current State**:
- ✅ Persistent storage implemented
- ✅ Docker multi-node setup ready
- ✅ 4-node configuration working
- ✅ Data persists across restarts
- ⏳ BFT consensus (Phase 2)
- ⏳ P2P networking (Phase 2)

**Next Steps**:
1. Implement P2P networking
2. Add BFT consensus algorithm
3. Implement state synchronization
4. Add network discovery

---

**Ready for Multi-Node Testing! 🎉**
