# 🚀 Miraset Chain - Complete User Guide

Welcome to Miraset Chain! This guide will walk you through everything you need to know to use the blockchain.

---

## Table of Contents

1. [What is Miraset Chain?](#what-is-miraset-chain)
2. [Installation](#installation)
3. [Starting a Node](#starting-a-node)
4. [Using the CLI](#using-the-cli)
5. [Using the TUI](#using-the-tui)
6. [RPC API Reference](#rpc-api-reference)
7. [Common Tasks](#common-tasks)
8. [Troubleshooting](#troubleshooting)

---

## What is Miraset Chain?

Miraset Chain is a blockchain designed for GPU compute sharing with:

- **Custom Consensus**: Proof of Compute & Capacity (PoCC)
- **Dual Rewards**: For GPU availability and compute work
- **On-Chain Chat**: Built-in messaging system
- **Fast Blocks**: 5-second block time
- **Easy to Use**: CLI and TUI interfaces

---

## Installation

### Prerequisites

You need:
- **Rust** 1.70 or newer ([Install from rustup.rs](https://rustup.rs))
- **Git** (to clone the repository)

### Build Steps

```bash
# 1. Clone the repository
git clone https://github.com/yourusername/miraset-chain.git
cd miraset-chain

# 2. Build all binaries (takes a few minutes)
cargo build --release

# 3. Binaries are now in target/release/
# - miraset (CLI tool)
# - miraset-tui (Terminal UI)
```

For faster development builds (but slower execution):
```bash
cargo build
# Binaries in target/debug/
```

---

## Starting a Node

### Quick Start

```bash
cargo run --bin miraset -- node start
```

You'll see:
```
Starting Miraset devnet node...
Genesis account: 4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd
Genesis secret: 0101010101010101010101010101010101010101010101010101010101010101
RPC listening on http://127.0.0.1:9944
```

**💡 The genesis account has a fixed address and secret for development!**

### What the Node Does

- ⚡ Produces blocks every 5 seconds
- 🌐 Runs RPC server on port 9944
- 💾 Stores state in memory (resets on restart)
- 📝 Logs block production to console

### Custom RPC Address

```bash
cargo run --bin miraset -- node start --rpc-addr 0.0.0.0:8080
```

---

## Using the CLI

The CLI is your main tool for interacting with the blockchain.

### Wallet Commands

#### Create New Account

```bash
cargo run --bin miraset -- wallet new alice
```

Output:
```
Created account 'alice': 82a7c778525562441c6b18652e0bd34440b4c6c3dfe8c9a6499c1588ed447b39
```

#### List All Accounts

```bash
cargo run --bin miraset -- wallet list
```

Output:
```
Accounts:
  alice -> 82a7c778525562441c6b18652e0bd34440b4c6c3dfe8c9a6499c1588ed447b39
  bob -> 1f3e2d4c5b6a7f8e9d0c1b2a3f4e5d6c7b8a9f0e1d2c3b4a5f6e7d8c9b0a1f2e
```

#### Check Balance

```bash
cargo run --bin miraset -- wallet balance alice
```

Output:
```
Balance for 'alice': 1000
```

#### Transfer Tokens

```bash
# Format: wallet transfer <from_name> <to_address> <amount>
cargo run --bin miraset -- wallet transfer alice 1f3e2d4c5b6a7f8e9d0c1b2a3f4e5d6c7b8a9f0e1d2c3b4a5f6e7d8c9b0a1f2e 500
```

Output:
```
Transfer submitted: alice -> 1f3e...f2e, amount: 500
```

**⏱️ Wait 5-10 seconds for the transaction to be included in a block!**

#### Import Account

```bash
cargo run --bin miraset -- wallet import myaccount <secret_key_hex>
```

For genesis:
```bash
cargo run --bin miraset -- wallet import genesis 0101010101010101010101010101010101010101010101010101010101010101
```

#### Export Secret Key

```bash
cargo run --bin miraset -- wallet export alice
```

Output:
```
Secret key for 'alice': a1b2c3d4e5f6...
WARNING: Keep this secret safe!
```

### Chat Commands

#### Send Message

```bash
cargo run --bin miraset -- chat send alice "Hello, Miraset!"
```

Output:
```
Chat message submitted
```

#### List Messages

```bash
cargo run --bin miraset -- chat list
```

Output:
```
Recent chat messages:
[2026-02-03 12:30:45 UTC] 82a7c778: Hello, Miraset!
[2026-02-03 12:31:02 UTC] 1f3e2d4c: This is cool!
```

With limit:
```bash
cargo run --bin miraset -- chat list --limit 100
```

### Node Commands

#### Start Node

```bash
cargo run --bin miraset -- node start [--rpc-addr 127.0.0.1:9944]
```

---

## Using the TUI

The Terminal UI provides an interactive interface.

### Launch TUI

```bash
cargo run --bin miraset-tui
```

### Navigation

| Key | Action |
|-----|--------|
| `1` | Switch to Wallet tab |
| `2` | Switch to Chat tab |
| `3` | Switch to Chain Info tab |
| `r` or `R` | Refresh data |
| `q` or `Q` | Quit application |

### Wallet Tab

Shows:
- List of all your accounts
- Current balances
- Account addresses

### Chat Tab

Features:
- **Message History**: Scrollable list of recent messages
- **Account Selector**: Use `↑`/`↓` to select sending account
- **Input Box**: Type your message
- **Send**: Press `Enter` to send

Example:
```
Selected Account: alice

Recent Messages:
[12:30] alice: Hello!
[12:31] bob: Hi there!

Your message: Type here and press Enter_
```

### Chain Info Tab

Displays:
- Current block height
- Latest block hash
- Recent blocks
- Transaction count

---

## RPC API Reference

The node exposes a REST API on `http://127.0.0.1:9944`.

### Endpoints

#### Get Balance

```bash
GET /balance/{address}
```

Example:
```bash
curl http://127.0.0.1:9944/balance/4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd
```

Response:
```json
1000000000000
```

#### Get Nonce

```bash
GET /nonce/{address}
```

Example:
```bash
curl http://127.0.0.1:9944/nonce/4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd
```

Response:
```json
5
```

#### Get Latest Block

```bash
GET /block/latest
```

Example:
```bash
curl http://127.0.0.1:9944/block/latest
```

Response:
```json
{
  "height": 42,
  "timestamp": "2026-02-03T12:30:45.123456Z",
  "prev_hash": [155, 43, ...],
  "transactions": [...],
  "state_root": [0, 0, ...]
}
```

#### Get Block by Height

```bash
GET /block/{height}
```

Example:
```bash
curl http://127.0.0.1:9944/block/10
```

#### Get Events

```bash
GET /events?from_height={height}&limit={limit}
```

Example:
```bash
curl "http://127.0.0.1:9944/events?from_height=0&limit=50"
```

Response:
```json
[
  {
    "event_type": "Transferred",
    "from": "4ae7bb...",
    "to": "82a7c7...",
    "amount": 1000,
    "tx_hash": [12, 34, ...],
    "block_height": 5
  },
  {
    "event_type": "ChatMessage",
    "from": "82a7c7...",
    "message": "Hello!",
    "tx_hash": [56, 78, ...],
    "block_height": 7,
    "timestamp": "2026-02-03T12:30:45Z"
  }
]
```

#### Get Chat Messages

```bash
GET /chat/messages?limit={limit}
```

Example:
```bash
curl "http://127.0.0.1:9944/chat/messages?limit=20"
```

Response:
```json
[
  {
    "from": "82a7c778...",
    "message": "Hello, Miraset!",
    "timestamp": "2026-02-03T12:30:45Z"
  }
]
```

#### Submit Transaction

```bash
POST /tx/submit
Content-Type: application/json
```

Example:
```bash
curl -X POST http://127.0.0.1:9944/tx/submit \
  -H "Content-Type: application/json" \
  -d '{
    "type": "Transfer",
    "from": "4ae7bb...",
    "to": "82a7c7...",
    "amount": 1000,
    "nonce": 0,
    "signature": "a1b2c3..."
  }'
```

Response: `200 OK` or `400 Bad Request`

---

## Common Tasks

### Task 1: Send Tokens from Genesis to New Account

```bash
# 1. Start node
cargo run --bin miraset -- node start &

# 2. Import genesis
cargo run --bin miraset -- wallet import genesis 0101010101010101010101010101010101010101010101010101010101010101

# 3. Create new account
cargo run --bin miraset -- wallet new alice

# 4. Get Alice's address
cargo run --bin miraset -- wallet list
# Copy alice's address

# 5. Transfer tokens
cargo run --bin miraset -- wallet transfer genesis <alice_address> 10000

# 6. Wait 5-10 seconds, then check
cargo run --bin miraset -- wallet balance alice
```

### Task 2: Have Two Accounts Chat

```bash
# 1. Create accounts
cargo run --bin miraset -- wallet new alice
cargo run --bin miraset -- wallet new bob

# 2. Fund them from genesis
cargo run --bin miraset -- wallet transfer genesis <alice_address> 1000
cargo run --bin miraset -- wallet transfer genesis <bob_address> 1000

# 3. Send messages
cargo run --bin miraset -- chat send alice "Hi Bob!"
sleep 6
cargo run --bin miraset -- chat send bob "Hi Alice!"
sleep 6

# 4. View conversation
cargo run --bin miraset -- chat list
```

### Task 3: Monitor Blockchain with TUI

```bash
# 1. Start node in background
cargo run --bin miraset -- node start > node.log 2>&1 &

# 2. Launch TUI
cargo run --bin miraset-tui

# 3. Press 'r' to refresh, navigate with 1/2/3
```

### Task 4: Query via RPC

```bash
# Get latest block
curl -s http://127.0.0.1:9944/block/latest | jq .

# Check balance
curl -s http://127.0.0.1:9944/balance/4ae7bb72c634332e0db3e27e22091bcd7a0167dafb5320e886d8c6ae21289ffd

# Get recent events
curl -s "http://127.0.0.1:9944/events?limit=10" | jq .
```

---

## Troubleshooting

### Problem: Port 9944 Already in Use

**Error**: `error 10048: Only one usage of each socket address...`

**Solution**: Another node is running. Stop it:

```bash
# Windows
taskkill /F /IM miraset.exe

# Linux/Mac
pkill miraset
```

### Problem: Transaction Not Showing Up

**Cause**: Blocks are produced every 5 seconds.

**Solution**: Wait at least 6 seconds after submitting a transaction before checking the result.

### Problem: Wallet Not Found

**Error**: `Error: Account 'alice' not found`

**Solution**: Create or import the account first:

```bash
cargo run --bin miraset -- wallet new alice
```

### Problem: Insufficient Balance

**Error**: `Insufficient balance`

**Solution**: Transfer tokens from genesis account:

```bash
cargo run --bin miraset -- wallet import genesis 0101010101010101010101010101010101010101010101010101010101010101
cargo run --bin miraset -- wallet transfer genesis <your_address> 10000
```

### Problem: Invalid Nonce

**Error**: `Invalid nonce: expected 5, got 3`

**Cause**: You submitted transactions out of order or the state was reset.

**Solution**: Query the current nonce and use it:

```bash
curl http://127.0.0.1:9944/nonce/<your_address>
```

### Problem: TUI Not Updating

**Solution**: Press `r` or `R` to manually refresh data.

### Problem: Can't Connect to RPC

**Error**: `Connection refused` or `Failed to connect`

**Solution**: Make sure the node is running:

```bash
cargo run --bin miraset -- node start
```

---

## Tips & Best Practices

### 💡 Development Tips

1. **Keep Node Running**: Start the node in one terminal, use CLI in another
2. **Wait for Blocks**: Always wait 5-10 seconds after submitting transactions
3. **Check Logs**: Node prints block production logs - watch for your transactions
4. **Use TUI for Monitoring**: The TUI auto-refreshes and is great for real-time monitoring

### 🔐 Security Tips

1. **Devnet Only**: The fixed genesis key is for development ONLY
2. **Backup Secrets**: Export and safely store your account secret keys
3. **Test First**: Try operations with small amounts before large transfers

### 🚀 Performance Tips

1. **Release Builds**: Use `cargo build --release` for faster execution
2. **Batch Transactions**: Multiple transactions in 5 seconds get batched in one block
3. **Local RPC**: The RPC server has minimal latency when accessed locally

---

## Where to Get Help

- **Documentation**: Check `docs/` directory for detailed specs
- **Source Code**: Browse `crates/` for implementation details
- **Issues**: Report bugs on GitHub Issues
- **Examples**: See `test_demo.sh` for complete workflow example

---

## Next Steps

Now that you know the basics:

1. Read [ARCHITECTURE.md](docs/ARCHITECTURE.md) to understand the system design
2. Check [REWARDS.md](docs/REWARDS.md) for the reward calculation formulas
3. Review [SOW.md](docs/SOW.md) for the complete specification
4. Explore the code in `crates/` to see how it works

---

**Happy Building! 🎉**

For the latest updates and features, check the [GitHub repository](https://github.com/yourusername/miraset-chain).
