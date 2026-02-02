## Miraset Chain

**Proof of Compute & Capacity (PoCC)** blockchain for GPU inference rewards.

### Why "miraset"?

`Mimir` — source of wisdom.  
`Ra` — ancient sun god, the power of origin.  
`Set` — the power of transformation.

---

## Quick Start

### Prerequisites
- Rust 1.70+ (`rustup`)
- Cargo

### Build
```bash
cargo build --release --workspace
```

Binaries will be in `target/release/`:
- `miraset` — CLI tool
- `miraset-tui` — Terminal UI

### Run Local Devnet

Start a local development node:
```bash
./target/release/miraset node start
```

This starts:
- Block producer (5-second blocks)
- RPC server on `http://127.0.0.1:9944`
- Genesis account with 1B tokens (secret key printed to console)

### CLI Usage

**Create wallet:**
```bash
./target/release/miraset wallet new alice
```

**Check balance:**
```bash
./target/release/miraset wallet balance alice
```

**Transfer tokens:**
```bash
./target/release/miraset wallet transfer alice <address> 1000
```

**Send chat message:**
```bash
./target/release/miraset chat send alice "Hello Miraset!"
```

**List chat:**
```bash
./target/release/miraset chat list
```

### Terminal UI

Run the TUI:
```bash
./target/release/miraset-tui
```

**Controls:**
- `1` — Wallet tab
- `2` — Chat tab
- `3` — Chain info tab
- `R` — Refresh data
- `Q` — Quit

Chat: type message and press Enter to send.

---

## Architecture

See `docs/`:
- `docs/SOW.md` — Full specification
- `docs/REWARDS.md` — Economic model & formulas
- `docs/ARCHITECTURE.md` — System design
- `docs/DATA.md` — Research notes

### MVP Features

✅ Local devnet (single-node BFT)  
✅ Wallet (keypairs, balances, transfers)  
✅ On-chain chat  
✅ CLI & TUI  
✅ Transaction signing & verification  
✅ Event indexing  

### Roadmap

- [ ] PoCC capacity attestation (VRAM + uptime)
- [ ] GPU job execution & rewards
- [ ] Batched epoch settlement
- [ ] ZK receipt anchoring
- [ ] Multi-node consensus
- [ ] Sui Move integration

---

## License

MIT

