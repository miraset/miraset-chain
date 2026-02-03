# 🚀 Quick Reference: Configuration & Persistence

## Start Node (Default Config)

```bash
cargo run --bin miraset -- node start
```

**Defaults:**
- RPC: `127.0.0.1:9944`
- Storage: `.data`
- Block interval: 5 seconds

---

## Configuration Options

### CLI Flags (Highest Priority)

```bash
# Custom storage path
cargo run --bin miraset -- node start --storage-path /my/data

# Custom RPC address
cargo run --bin miraset -- node start --rpc-addr 0.0.0.0:9944

# Custom block interval
cargo run --bin miraset -- node start --block-interval 10

# All combined
cargo run --bin miraset -- node start \
  --rpc-addr 0.0.0.0:9944 \
  --storage-path /var/lib/miraset \
  --block-interval 3
```

### Config File

Create `miraset.toml` in project root:

```toml
[node]
rpc_addr = "127.0.0.1:9944"
storage_path = ".data"
block_interval = 5
```

### Environment Variables

```bash
export MIRASET_RPC_ADDR="127.0.0.1:9944"
export MIRASET_STORAGE_PATH=".data"
export MIRASET_BLOCK_INTERVAL="5"
```

---

## Precedence

```
CLI Flags > Environment Variables > Config File > Defaults
```

---

## Common Use Cases

### Development (Default)

```bash
cargo run --bin miraset -- node start
# Uses: .data, 127.0.0.1:9944, 5s blocks
```

### Fast Testing

```bash
cargo run --bin miraset -- node start --block-interval 1
# Produces blocks every second
```

### Temporary Test Node

```bash
cargo run --bin miraset -- node start --storage-path ./tmp_test
# Use temporary storage, delete after testing
```

### Production

Create `miraset.toml`:
```toml
[node]
rpc_addr = "0.0.0.0:9944"
storage_path = "/var/lib/miraset/data"
block_interval = 3
```

Then:
```bash
cargo run --release --bin miraset -- node start
```

---

## Data Persistence

### What Gets Saved

✅ All blocks (genesis to latest)  
✅ All account balances  
✅ All account nonces  
✅ All events (transfers, chat, worker registrations)  

### Storage Location

Default: `.data/` in current directory

Check storage:
```bash
ls -lah .data/
du -sh .data/
```

### Restart Behavior

1. Node shuts down → Data saved to `.data/`
2. Node restarts → Data loaded from `.data/`
3. Block production continues from last height

### Data Management

```bash
# Backup
cp -r .data .data.backup

# Fresh start (delete all data)
rm -rf .data

# Migrate to new location
mv .data /new/location
cargo run --bin miraset -- node start --storage-path /new/location
```

---

## Testing Persistence

```bash
# Terminal 1: Start node
cargo run --bin miraset -- node start

# Wait for blocks...
# Stop with Ctrl+C

# Check data exists
ls .data/

# Restart
cargo run --bin miraset -- node start

# Terminal 2: Verify blocks persisted
curl http://127.0.0.1:9944/block/1
curl http://127.0.0.1:9944/block/latest
```

---

## Help

```bash
# General help
cargo run --bin miraset -- --help

# Node help
cargo run --bin miraset -- node --help

# Start help
cargo run --bin miraset -- node start --help
```

---

## Documentation

- **[CONFIGURATION.md](CONFIGURATION.md)** - Full configuration guide
- **[PERSISTENCE.md](PERSISTENCE.md)** - Persistence details
- **[README.md](README.md)** - Getting started
