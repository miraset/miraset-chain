# Miraset Chain — Quick Test Guide

## 1. Start the devnet node

In terminal 1:
```bash
cargo run --bin miraset -- node start
```

Look for output like:
```
Genesis account: abc123...
Genesis secret: def456...
RPC listening on http://127.0.0.1:9944
```

**Save the genesis secret key** — you'll need it to fund other accounts.

## 2. Create and fund wallets

In terminal 2:

```bash
# Create two accounts
cargo run --bin miraset -- wallet new alice
cargo run --bin miraset -- wallet new bob

# Import genesis account (use the secret from step 1)
cargo run --bin miraset -- wallet import genesis <secret_key_here>

# Transfer tokens from genesis to alice
cargo run --bin miraset -- wallet transfer genesis <alice_address> 100000

# Check alice's balance
cargo run --bin miraset -- wallet balance alice
```

## 3. Test chat

```bash
# Send a message
cargo run --bin miraset -- chat send alice "Hello from Alice!"

# Send more messages
cargo run --bin miraset -- chat send alice "Testing the chain"

# List all messages
cargo run --bin miraset -- chat list
```

## 4. Test transfers

```bash
# Transfer from alice to bob
cargo run --bin miraset -- wallet transfer alice <bob_address> 500

# Check balances
cargo run --bin miraset -- wallet balance alice
cargo run --bin miraset -- wallet balance bob
```

## 5. Run the TUI

In terminal 2:
```bash
cargo run --bin miraset-tui
```

Press `1`, `2`, `3` to switch tabs.  
In chat tab, type and press Enter to send.  
Press `R` to refresh, `Q` to quit.

## Expected behavior

✅ Blocks produced every 5 seconds  
✅ Transactions appear in next block  
✅ Balances update immediately  
✅ Chat messages persist  
✅ TUI auto-refreshes every 3 seconds

## Troubleshooting

**"Connection refused"**  
→ Make sure node is running (step 1)

**"Invalid nonce"**  
→ Each transaction increments nonce. If you restarted the node, old wallet state is stale. Create new accounts.

**"Insufficient balance"**  
→ Fund account from genesis first

**TUI shows empty data**  
→ Wait 3 seconds for auto-refresh or press `R`
