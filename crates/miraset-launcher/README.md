# MIRASET Launcher (EXE)

Small native launcher that starts the node, worker, and wallet GUI from the same folder.

## Build

From the repo root:

```bash
cargo build --release --bin miraset-launcher
```

The resulting binary is copied into `build/` by the packaging scripts.

## Run

Place these files in the same folder:
- `miraset-launcher.exe`
- `miraset.exe`
- `miraset-worker.exe`
- `wallet-miraset.exe`

Then run:

```bash
miraset-launcher.exe
```

Press Enter or `Ctrl+C` to stop all services.

