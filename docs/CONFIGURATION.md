# ⚙️ Configuration Guide

## Overview

Miraset Chain supports flexible configuration through multiple sources with clear precedence rules.

**Precedence (highest to lowest):**
1. **CLI flags** - Command-line arguments
2. **Environment variables** - `MIRASET_*` prefixed vars
3. **Config file** - `miraset.toml` in project root
4. **Defaults** - Built-in sensible defaults

---

## Configuration Sources

### 1. CLI Flags (Highest Priority)

Override any setting directly from the command line:

```bash
# Start node with custom settings
cargo run --bin miraset -- node start \
  --rpc-addr 0.0.0.0:9944 \
  --storage-path /custom/path \
  --block-interval 10
```

**Available Flags:**

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--rpc-addr` | String | `127.0.0.1:9944` | RPC server bind address |
| `--storage-path` | String | `.data` | Path to persistent storage |
| `--block-interval` | u64 | `5` | Block production interval (seconds) |

### 2. Environment Variables

Set environment variables with `MIRASET_` prefix:

```bash
# Linux/macOS
export MIRASET_RPC_ADDR="0.0.0.0:9944"
export MIRASET_STORAGE_PATH="/var/lib/miraset"
export MIRASET_BLOCK_INTERVAL="10"

# Windows (PowerShell)
$env:MIRASET_RPC_ADDR="0.0.0.0:9944"
$env:MIRASET_STORAGE_PATH="C:\miraset\data"
$env:MIRASET_BLOCK_INTERVAL="10"

# Windows (CMD)
set MIRASET_RPC_ADDR=0.0.0.0:9944
set MIRASET_STORAGE_PATH=C:\miraset\data
set MIRASET_BLOCK_INTERVAL=10
```

Then start the node:

```bash
cargo run --bin miraset -- node start
```

### 3. Config File (Auto-Discovered)

Create `miraset.toml` in the project root directory. The file is **auto-discovered** and loaded silently if present.

**Example `miraset.toml`:**

```toml
[node]
# RPC server address
rpc_addr = "127.0.0.1:9944"

# Storage path (relative to project root)
storage_path = ".data"

# Block production interval in seconds
block_interval = 5
```

**Location:**
- File must be named `miraset.toml`
- Must be in the project root (same directory as `Cargo.toml`)
- Silent if missing - no errors or warnings

### 4. Defaults

If no configuration is provided, these defaults are used:

| Setting | Default Value |
|---------|---------------|
| `rpc_addr` | `127.0.0.1:9944` |
| `storage_path` | `.data` |
| `block_interval` | `5` |

---

## Configuration Examples

### Example 1: Development Setup

**Config:** Default values
```bash
cargo run --bin miraset -- node start
```

**Result:**
- RPC: `127.0.0.1:9944`
- Storage: `.data/`
- Block interval: 5 seconds

### Example 2: Production Setup

**Config file** (`miraset.toml`):
```toml
[node]
rpc_addr = "0.0.0.0:9944"
storage_path = "/var/lib/miraset/data"
block_interval = 3
```

```bash
cargo run --release --bin miraset -- node start
```

**Result:**
- RPC: `0.0.0.0:9944` (binds to all interfaces)
- Storage: `/var/lib/miraset/data`
- Block interval: 3 seconds

### Example 3: Testing with Custom Path

**CLI override:**
```bash
cargo run --bin miraset -- node start \
  --storage-path ./test_data \
  --block-interval 1
```

**Result:**
- RPC: `127.0.0.1:9944` (default)
- Storage: `./test_data`
- Block interval: 1 second (fast testing)

### Example 4: Docker Production

**Environment variables in docker-compose.yml:**
```yaml
services:
  node:
    environment:
      - MIRASET_RPC_ADDR=0.0.0.0:9944
      - MIRASET_STORAGE_PATH=/data
      - MIRASET_BLOCK_INTERVAL=5
```

### Example 5: Multiple Overrides

**Config file** (`miraset.toml`):
```toml
[node]
rpc_addr = "127.0.0.1:9944"
storage_path = ".data"
block_interval = 5
```

**Environment:**
```bash
export MIRASET_BLOCK_INTERVAL="3"
```

**CLI:**
```bash
cargo run --bin miraset -- node start --storage-path /tmp/test
```

**Result (precedence applied):**
- RPC: `127.0.0.1:9944` (from config file)
- Storage: `/tmp/test` (from CLI - highest priority)
- Block interval: 3 seconds (from env - overrides config)

---

## Storage Path Options

### Relative Paths

Relative to the current working directory:

```bash
# Store in .data subdirectory
--storage-path .data

# Store in parent directory
--storage-path ../shared_data

# Store in temp
--storage-path ./tmp/test_node
```

### Absolute Paths

**Linux/macOS:**
```bash
--storage-path /var/lib/miraset
--storage-path /home/user/miraset_data
```

**Windows:**
```bash
--storage-path C:\miraset\data
--storage-path D:\blockchain\miraset
```

### Special Considerations

- **Permissions**: Ensure the process has read/write access
- **Disk Space**: Blockchain data grows over time
- **Backup**: Store in a location that's backed up
- **Performance**: SSD recommended for better performance

---

## RPC Address Options

### Local Development

Bind to localhost only (secure, not accessible from network):

```bash
--rpc-addr 127.0.0.1:9944
```

### Production/Network Access

Bind to all interfaces (accessible from network):

```bash
--rpc-addr 0.0.0.0:9944
```

⚠️ **Security Warning**: Binding to `0.0.0.0` exposes RPC to the network. Use firewall rules or reverse proxy for production.

### Custom Port

```bash
--rpc-addr 127.0.0.1:8545  # Ethereum-style
--rpc-addr 127.0.0.1:3000  # Alternative
```

---

## Block Interval Options

Controls how often blocks are produced (in seconds).

### Fast Testing

```bash
--block-interval 1  # 1 block per second
```

### Development

```bash
--block-interval 5  # Default, good balance
```

### Production

```bash
--block-interval 3  # Faster confirmations
--block-interval 10 # More stable, less CPU
```

**Trade-offs:**
- **Lower values** (1-3s): Faster transactions, higher CPU usage
- **Higher values** (10-30s): Lower resource usage, slower confirmations

---

## Verification

### Check Active Configuration

When you start the node, it prints the active configuration:

```bash
$ cargo run --bin miraset -- node start
Starting Miraset devnet node...
RPC address: 127.0.0.1:9944
Storage path: .data
Block interval: 5s
Storage opened at: .data
Genesis account: 8f4d3e2a1b9c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c3d2e
Genesis secret: 0101010101010101010101010101010101010101010101010101010101010101
RPC listening on http://127.0.0.1:9944
```

### Test Configuration

```bash
# Test RPC is accessible
curl http://127.0.0.1:9944/block/latest

# Check storage directory exists
ls -la .data/

# Monitor block production
watch -n 1 'curl -s http://127.0.0.1:9944/block/latest | jq .height'
```

---

## Troubleshooting

### Config File Not Loading

**Problem**: Config file seems ignored

**Solution**:
1. Ensure file is named exactly `miraset.toml`
2. Place in project root (same directory as `Cargo.toml`)
3. Check TOML syntax is valid
4. Config file failures are silent - check for syntax errors

### Storage Path Permission Denied

**Problem**: `Error: Permission denied (os error 13)`

**Solution**:
```bash
# Linux/macOS: Fix permissions
chmod 755 /path/to/storage

# Or use a path in your home directory
--storage-path ~/.miraset/data
```

### Port Already in Use

**Problem**: `Error: Address already in use (os error 48)`

**Solution**:
```bash
# Use a different port
--rpc-addr 127.0.0.1:9945

# Or kill the existing process
lsof -ti:9944 | xargs kill
```

### Storage Path Not Created

**Problem**: Storage directory doesn't exist

**Solution**:
Sled creates the directory automatically, but parent directories must exist:

```bash
# Wrong: /nonexistent/data won't work
--storage-path /nonexistent/data

# Right: Create parent first
mkdir -p /path/to/parent
--storage-path /path/to/parent/data
```

---

## Best Practices

### Development

```toml
[node]
rpc_addr = "127.0.0.1:9944"
storage_path = ".data"
block_interval = 5
```

### Testing

```bash
# Use temporary storage that can be deleted
cargo run --bin miraset -- node start \
  --storage-path ./tmp/test_$(date +%s) \
  --block-interval 1
```

### Production

```toml
[node]
rpc_addr = "0.0.0.0:9944"
storage_path = "/var/lib/miraset/data"
block_interval = 3
```

Use systemd or similar to manage the process:

```ini
[Unit]
Description=Miraset Node
After=network.target

[Service]
Type=simple
User=miraset
Environment=MIRASET_STORAGE_PATH=/var/lib/miraset/data
ExecStart=/usr/local/bin/miraset node start
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

### Docker

Use environment variables in `docker-compose.yml`:

```yaml
version: '3.8'

services:
  node:
    image: miraset-chain
    environment:
      - MIRASET_RPC_ADDR=0.0.0.0:9944
      - MIRASET_STORAGE_PATH=/data
      - MIRASET_BLOCK_INTERVAL=5
    volumes:
      - node-data:/data
    ports:
      - "9944:9944"

volumes:
  node-data:
```

---

## Migration Guide

### From Older Versions

If you were using `./data` as the storage path, no action needed - just configure:

```toml
[node]
storage_path = "./data"  # Keep using old path
```

Or migrate the data:

```bash
# Backup old data
cp -r ./data .data

# Start with new path (default)
cargo run --bin miraset -- node start
```

### Changing Storage Path

To move your blockchain data:

```bash
# Stop the node first
# Move data
mv .data /new/path

# Start with new path
cargo run --bin miraset -- node start --storage-path /new/path
```

---

## See Also

- [PERSISTENCE.md](PERSISTENCE.md) - Storage internals
- [DOCKER.md](DOCKER.md) - Docker deployment
- [README.md](../README.md.md) - Quick start guide
