# MIRASET Wallet Desktop Backend

This folder contains the Tauri backend for `wallet-miraset.exe`.

## Commands

- `get_config` / `set_rpc_url`
- `list_accounts`, `create_account`, `import_account`, `export_secret`
- `get_balance`, `transfer`

## Build

From `wallet/`:

```bash
bun install
bun run tauri:build
```

