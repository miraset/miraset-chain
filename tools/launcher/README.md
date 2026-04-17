# MIRASET Launcher

Launch and package the node, worker, and wallet GUI from scripts (no Python).

## Requirements
- Rust toolchain (`cargo`)
- Bun (`bunx`) for the wallet GUI build
- PowerShell (for ZIP on Windows) or `zip` in your shell

## Build + Package

```bat
tools\launcher\build-and-package.bat
```

```bash
./tools/launcher/build-and-package.sh
```

Artifacts are copied into `build/` and zipped into `releases/miraset-windows.zip`.

## Launch (EXE)

After packaging, run the launcher EXE from `build/`:

```bat
build\miraset-launcher.exe
```

Press Enter or `Ctrl+C` in the launcher window to stop all services.

## Launch (scripts, uses built binaries)

```bat
tools\launcher\launch.bat
```

```bash
./tools/launcher/launch.sh
```

## Options
- For release builds, use the build script (it already uses `--release`).
- Use `Ctrl+C` to stop services when running from `launch.sh`.
