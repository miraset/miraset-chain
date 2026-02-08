# 🐳 Docker Setup for Multi-Node Miraset Chain

## Quick Start

### Build and Run 4 Nodes

```bash
# Build images
docker-compose build

# Start all 4 nodes
docker-compose up -d

# View logs
docker-compose logs -f

# Stop all nodes
docker-compose down

# Clean up (including volumes)
docker-compose down -v
```

## Node Configuration

### Ports
- **Node 1**: `localhost:9944` → `172.20.0.11:9944`
- **Node 2**: `localhost:9945` → `172.20.0.12:9944`
- **Node 3**: `localhost:9946` → `172.20.0.13:9944`
- **Node 4**: `localhost:9947` → `172.20.0.14:9944`

### Network
- **Network**: `miraset-net` (172.20.0.0/16)
- **Driver**: bridge
- **DNS**: Automatic container name resolution

### Volumes
Each node has persistent storage:
- `node1-data` → `/data` (in container)
- `node2-data` → `/data`
- `node3-data` → `/data`
- `node4-data` → `/data`

## Testing Multi-Node Setup

### Test Individual Nodes

```bash
# Test node 1
curl http://localhost:9944/block/latest

# Test node 2
curl http://localhost:9945/block/latest

# Test node 3
curl http://localhost:9946/block/latest

# Test node 4
curl http://localhost:9947/block/latest
```

### Run RPC Tests on Each Node

```bash
# Test all nodes
for port in 9944 9945 9946 9947; do
    echo "Testing node on port $port..."
    curl -s http://localhost:$port/block/latest | head -20
    echo ""
done
```

### Check Node Status

```bash
# View running containers
docker ps

# View logs for specific node
docker logs miraset-node1
docker logs miraset-node2 -f  # Follow logs

# Execute commands in container
docker exec -it miraset-node1 bash

# View resource usage
docker stats
```

## Commands

### Start/Stop Individual Nodes

```bash
# Start single node
docker-compose up -d node1

# Stop single node
docker-compose stop node1

# Restart node
docker-compose restart node1

# Remove node (keeps data)
docker-compose rm -f node1
```

### Scaling

```bash
# Scale to 6 nodes (requires config update)
docker-compose up -d --scale node=6
```

### Logs

```bash
# All logs
docker-compose logs

# Follow logs
docker-compose logs -f

# Last 100 lines
docker-compose logs --tail=100

# Specific node
docker-compose logs node1

# With timestamps
docker-compose logs -t
```

## Persistent Storage

### Data Location

Data is stored in Docker volumes:
```bash
# List volumes
docker volume ls

# Inspect volume
docker volume inspect miraset-chain_node1-data

# Backup volume
docker run --rm -v miraset-chain_node1-data:/data -v $(pwd):/backup ubuntu tar czf /backup/node1-backup.tar.gz /data

# Restore volume
docker run --rm -v miraset-chain_node1-data:/data -v $(pwd):/backup ubuntu tar xzf /backup/node1-backup.tar.gz -C /
```

### Clean Up Storage

```bash
# Remove all data (DESTRUCTIVE!)
docker-compose down -v

# Remove specific volume
docker volume rm miraset-chain_node1-data
```

## Development Workflow

### Local Development with Docker

```bash
# 1. Make code changes locally
vim crates/miraset-node/src/state.rs

# 2. Rebuild specific service
docker-compose build node1

# 3. Restart with new image
docker-compose up -d node1

# 4. Check logs
docker-compose logs -f node1
```

### Hot Reload (Optional)

Mount local code into container:
```yaml
volumes:
  - ./target:/app/target
  - ./crates:/app/crates
```

## Network Communication

### Container-to-Container

Nodes can communicate via container names:
```bash
# From inside node1 container
curl http://miraset-node2:9944/block/latest
```

### Host-to-Container

From host machine:
```bash
# Via localhost ports
curl http://localhost:9944/block/latest  # node1
curl http://localhost:9945/block/latest  # node2
```

## Troubleshooting

### Port Already in Use

```bash
# Find process using port
netstat -ano | findstr :9944  # Windows
lsof -i :9944                 # Linux/Mac

# Kill process
taskkill /PID <pid> /F        # Windows
kill -9 <pid>                 # Linux/Mac
```

### Container Won't Start

```bash
# Check logs
docker-compose logs node1

# Check container status
docker ps -a

# Remove and recreate
docker-compose rm -f node1
docker-compose up -d node1
```

### Network Issues

```bash
# Inspect network
docker network inspect miraset-chain_miraset-net

# Test connectivity
docker exec -it miraset-node1 ping miraset-node2

# Recreate network
docker-compose down
docker-compose up -d
```

### Build Issues

```bash
# Clean build
docker-compose build --no-cache

# Check Docker disk space
docker system df

# Clean up unused images
docker system prune -a
```

## Production Considerations

### Environment Variables

Add to `.env` file:
```env
RUST_LOG=info
NODE_ID=1
GENESIS_SECRET=your_secret_here
ENABLE_METRICS=true
```

### Resource Limits

Add to docker-compose.yml:
```yaml
deploy:
  resources:
    limits:
      cpus: '2'
      memory: 2G
    reservations:
      cpus: '1'
      memory: 1G
```

### Health Checks

```yaml
healthcheck:
  test: ["CMD", "curl", "-f", "http://localhost:9944/block/latest"]
  interval: 30s
  timeout: 10s
  retries: 3
  start_period: 40s
```

### Restart Policy

```yaml
restart: unless-stopped
```

## Monitoring

### Prometheus Integration (Future)

```yaml
prometheus:
  image: prom/prometheus
  ports:
    - "9090:9090"
  volumes:
    - ./prometheus.yml:/etc/prometheus/prometheus.yml
```

### Grafana Dashboard (Future)

```yaml
grafana:
  image: grafana/grafana
  ports:
    - "3000:3000"
  environment:
    - GF_SECURITY_ADMIN_PASSWORD=admin
```

## Security

### Best Practices

1. **Don't use default genesis key in production**
2. **Use secrets management** (Docker Secrets, Vault)
3. **Enable TLS** for RPC endpoints
4. **Restrict network access** (firewall rules)
5. **Regular backups** of volumes
6. **Monitor logs** for suspicious activity

### Example with Secrets

```yaml
secrets:
  genesis_key:
    file: ./secrets/genesis.key

services:
  node1:
    secrets:
      - genesis_key
```

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Docker Build

on: [push]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - name: Build images
        run: docker-compose build
      - name: Run tests
        run: docker-compose up -d && sleep 10 && ./test_docker_nodes.sh
```

## Commands Cheat Sheet

```bash
# Build
docker-compose build
docker-compose build --no-cache

# Start
docker-compose up -d
docker-compose up -d node1 node2

# Stop
docker-compose down
docker-compose stop

# Logs
docker-compose logs -f
docker-compose logs --tail=100 node1

# Status
docker-compose ps
docker ps

# Exec
docker exec -it miraset-node1 bash
docker exec miraset-node1 miraset wallet list

# Clean
docker-compose down -v
docker system prune -a

# Backup
docker run --rm -v node1-data:/data -v $(pwd):/backup ubuntu tar czf /backup/backup.tar.gz /data
```

---

**Ready for BFT Consensus Testing! 🚀**
