# MIRASET Wallet — Design Report

> Scope: the desktop wallet under `wallet/` (Next.js UI + Tauri/Rust backend) and the shared
> keystore crate `crates/miraset-wallet/`. All file references are relative to the repo root.

## 1. What it is

The MIRASET wallet is a **Tauri 2 desktop application** that wraps a Next.js (App Router, React 19)
single-page UI in a native `wallet-miraset.exe`. It is the user-facing client for the MIRASET
chain:

- manages local accounts (create / import / export / list),
- signs and submits `Transfer` and inference `SubmitJob` transactions,
- reads chain state (balances, nonces, events, workers, jobs, epoch),
- surfaces health of three local services: MIRASET RPC, MIRASET worker, and Ollama.

The Rust backend (in `wallet/src-tauri/`) is intentionally thin: it owns the keystore, persists
app config, and brokers HTTP calls to the node. The UI never talks to the chain directly; it
invokes Tauri commands and receives JSON.

```
+------------------------------------------------------------+
|  BrowserView (React 19, Next.js, Tailwind v4)              |
|  wallet/src/app/page.tsx                                   |
+----------------------------+-------------------------------+
                             |  invoke() (Tauri IPC)
                             v
+------------------------------------------------------------+
|  Tauri 2 backend (Rust)                                    |
|  wallet/src-tauri/src/main.rs                              |
|  - keystore via miraset-wallet crate                       |
|  - reqwest client to node RPC + worker + ollama            |
+----------------------------+-------------------------------+
                             |
                             v
+------------------------------------------------------------+
|  Local services (out-of-process)                           |
|  - miraset node  (RPC, default http://127.0.0.1:9944)      |
|  - miraset-worker (default http://127.0.0.1:8080)          |
|  - ollama         (default http://127.0.0.1:11434)         |
+------------------------------------------------------------+
```

## 2. High-level architecture

### 2.1 Process model

A single Tauri process owns:

- the WebView running the Next.js bundle (`wallet/out/` produced by `next build`),
- the Rust command handlers registered via `tauri::generate_handler!` in
  `wallet/src-tauri/src/main.rs:319-335`.

The Next.js dev server is only used in dev mode (`tauri:dev` → `bunx tauri dev` →
`beforeDevCommand: bun run dev` in `tauri.conf.json:7`). Production builds embed the static
`out/` directory (`frontendDist: "../out"`).

### 2.2 Trust boundary

The WebView is treated as untrusted UI. All sensitive operations (key access, signing, raw
HTTP) run in the Rust process. The UI sends intent over IPC and receives redacted
results (e.g. `export_secret` returns the secret key hex to be displayed, which is a known
limitation — see §7).

### 2.3 External services

Three URLs are user-configurable from the UI (RPC) and otherwise hard-coded in the UI module
as defaults (`wallet/src/app/page.tsx:6-8`):

- `RPC_URL` → `http://127.0.0.1:9944` (persisted in `config.json`),
- `WORKER_URL` → `http://127.0.0.1:8080` (used only for `/health` probing),
- `OLLAMA_URL` → `http://127.0.0.1:11434` (used only for `/api/tags` probing).

The `WORKER_URL` and `OLLAMA_URL` are currently not user-editable; only `rpc_url` is
(`handleUpdateRpc` → `set_rpc_url` command, `wallet/src/app/page.tsx:469-492`).

## 3. Tauri command surface

All commands are defined in `wallet/src-tauri/src/main.rs`. They are registered in
`main()` at lines 319-335.

| Command | Signature | Purpose |
|---|---|---|
| `get_config` | `() -> AppConfig` | Load persisted `rpc_url` + `wallet_path`. |
| `set_rpc_url` | `(rpc_url: String) -> AppConfig` | Persist a new RPC URL. |
| `list_accounts` | `() -> Vec<AccountView>` | Enumerate local accounts (`name`, `address`). |
| `create_account` | `(name: String) -> String` | Generate a new ed25519 keypair, store it. |
| `import_account` | `(name: String, secret_hex: String) -> String` | Import a 32-byte secret. |
| `export_secret` | `(name: String) -> String` | Return the raw 32-byte secret as hex. |
| `get_balance` | `(address: String) -> u64` | `GET {rpc}/balance/{addr}`. |
| `transfer` | `(from, to: String, amount: u64) -> ()` | Build, sign, and submit a `Transfer` tx. |
| `check_connections` | `(rpc, worker, ollama: String) -> ConnectionSnapshot` | Probe each `/health` (or `/api/tags`). |
| `get_events` | `(from_height, limit) -> Vec<Value>` | `GET {rpc}/events?...`. |
| `get_workers` | `() -> Vec<Value>` | `GET {rpc}/workers`. |
| `submit_job` | `(from, model_id, max_tokens, escrow_amount) -> Value` | `POST {rpc}/jobs/submit`. |
| `get_epoch` | `() -> Value` | `GET {rpc}/epoch`. |
| `get_jobs` | `() -> Vec<Value>` | `GET {rpc}/jobs`. |

`AccountView` deliberately omits `balance` — balances are fetched separately per account so
the UI can refresh them independently (see §5.2).

## 4. Keystore design (`crates/miraset-wallet/`)

The keystore is a small library reused by the Tauri backend, the `miraset-cli`, and any
future Rust-side signer. It is intentionally separate from the chain RPC layer.

### 4.1 On-disk format

A wallet file is a JSON document of one of two shapes (`crates/miraset-wallet/src/lib.rs:20-32`):

```jsonc
// Plaintext (legacy, still default when no password is set)
{
  "accounts": { "alice": "<hex 32 bytes>", "bob": "<hex 32 bytes>" }
}

// Encrypted envelope (chosen when a password is set)
{
  "version": 1,
  "salt":     "<hex 16 bytes>",   // Argon2id salt
  "nonce":    "<hex 12 bytes>",   // AES-256-GCM nonce
  "ciphertext": "<base64 std>"    // ciphertext of WalletData JSON
}
```

File location: `~/.miraset/wallet.json` on Unix, `%USERPROFILE%\.miraset\wallet.json` on
Windows. The path is computed in `default_wallet_path()` in
`wallet/src-tauri/src/main.rs:48-53` and is overridable via `AppConfig.wallet_path`.

### 4.2 Crypto choices

- **KDF**: Argon2id with default parameters (`Argon2::default()`), 16-byte random salt per
  file (`derive_key` in `lib.rs:199-205`).
- **AEAD**: AES-256-GCM with a 12-byte random nonce per save (`encrypt_wallet_data` /
  `decrypt_wallet_data` in `lib.rs:208-259`).
- **Keys**: 32-byte ed25519 secret keys, generated by `KeyPair::generate()` in
  `crates/miraset-core/src/crypto.rs:54-63` using `OsRng`.
- **Addresses**: 32-byte ed25519 public key (raw, not hashed) — `Address` wraps
  `VerifyingKey::as_bytes()` (`crypto.rs:74-76`). Display truncates to 8 hex chars
  (`crypto.rs:41-44`).

### 4.3 API surface

`Wallet` (`lib.rs:34-196`) is the only struct callers need:

- `Wallet::new(path)` — plaintext wallet; refuses to open an encrypted file.
- `Wallet::open_encrypted(path, password)` — opens an encrypted file; if the file is
  plaintext it transparently upgrades to encrypted on the first save
  (`lib.rs:67-103`).
- `create_account(name)` / `import_account(name, secret_hex)` / `get_keypair(name)` /
  `list_accounts()` / `export_secret(name)`.
- `set_password(p)` / `remove_password()` toggle encryption on next save.

Mutating calls auto-save. There is no in-memory write batching, and no `flush()` —
`fs::write` is used directly. The unit tests at `lib.rs:261-591` cover create/import/export
round-trips, duplicate names, and the encryption lifecycle (including the
"plaintext cannot open encrypted" guard at `lib.rs:551-565`).

### 4.4 Sign flow

The Tauri `transfer` command (`main.rs:151-179`) is the only place that signs a tx in the
wallet. It follows the project's standard pattern (zero the signature field, bincode the
struct, sign the bytes, paste the signature back), which is the same one used in
`miraset-cli`. The resulting JSON is POSTed to `/tx/submit`.

## 5. UI design (`wallet/src/app/`)

### 5.1 Layout & styling

- Single-page App Router at `wallet/src/app/page.tsx` (1097 lines, `"use client"`).
- Tailwind v4 with a dark, monochrome palette and a single orange accent
  (`--accent: #f7931a`, the Bitcoin "orange") — see `wallet/src/app/globals.css:3-9`.
- Display font: `Bebas Neue`; body: `Ubuntu`, both loaded via `next/font` in
  `wallet/src/app/layout.tsx`.
- Two responsive breakpoints: `md` (single-column → two-column), `lg` (1.1fr / 1fr split).

### 5.2 State management

Pure React hooks, no global store:

- `useState` for everything per-component.
- `useSyncExternalStore` for two read-only, app-wide stores:
  - **Connection snapshot** (`subscribeConnections` at `page.tsx:116-127`) — shared
    polling state. The store lazily starts a 10 s `setInterval` when the first subscriber
    mounts and stops it on the last unsubscribe.
  - **Tauri detection / hydration** — tiny adapters that return stable client vs server
    values to avoid SSR/WebView mismatches.
- `useRef` only for the `refreshTimerRef` interval handle (`page.tsx:274-296`).
- A 10 s `setInterval` (`page.tsx:275-296`) re-fetches balances whenever accounts are
  loaded. The connection poller above is independent and reports RPC/worker/Ollama health
  via a coloured-dot UI.

There is no reducer, no Context provider, no React Query. Every command goes through a
shared `tauriInvoke` wrapper (`page.tsx:196-204`) that throws a friendly error if the WebView
is not running inside Tauri.

### 5.3 Sections

Top to bottom, the page renders:

1. **Header** with status pills (RPC / worker / Ollama + last-checked timestamp).
2. **Offline banner** for the RPC and a softer warning for the worker.
3. **RPC config card** (editable URL, save, refresh, full status detail, wallet path).
4. **Accounts card** (list + per-account balance) with:
   - create-account form (name → `create_account`),
   - import-account form (name + secret hex → `import_account`).
5. **Selected account card** (address, balance, copy, export-secret).
6. **Send / Receive cards** (transfer form, receive address).
7. **Transaction history** (events from `/events`, parsed heuristically — see §7).
8. **Submit inference job** (model, max tokens, escrow) + **Workers** + **Jobs** panels.

The default for an empty `accounts` list is to **not** call `list_accounts` automatically.
The user clicks "Load existing wallets" (`page.tsx:715-725`) which flips `accountsLoaded`
and triggers the 10 s balance refresh. This is a deliberate privacy default — the wallet
file is read only on demand.

### 5.4 Tauri detection & SSR safety

`detectTauri()` (`page.tsx:146-151`) checks for `window.__TAURI_INTERNALS__`. The
hydration path returns a "loading" shell; the no-Tauri path returns a single card
instructing the user to run `bunx tauri dev` (`page.tsx:515-527`). This means the same
bundle works in `next dev` (browser-only, no IPC) and inside the desktop app.

## 6. Build, package, distribution

| Step | Command (from `wallet/`) | Output |
|---|---|---|
| Install deps | `bun install` | `bun.lock` |
| Dev (Tauri) | `bun run tauri:dev` | Hot-reloading WebView + dev server |
| Prod UI build | `bun run build:ui` | `wallet/out/` (static Next.js export) |
| Prod app build | `bun run tauri:build` | `wallet/src-tauri/target/release/wallet-miraset[.exe]` |
| ZIP bundle | `bun run tauri:build && node scripts/package-zip.mjs` | `wallet/dist/wallet-miraset-*.zip` |

`scripts/package-zip.mjs` has a `--dry-run` mode (`pack:zip:dry`) that lists what would
be archived without writing. The top-level `tools/launcher/build-and-package.{sh,bat}`
scripts drive the same flow end-to-end for the desktop launcher.

The Tauri config (`wallet/src-tauri/tauri.conf.json`) sets `bundle.active = false`
(line 26), deferring bundling to the package script. CSP is `null` in dev for ease of
iteration.

## 7. Known limitations and sharp edges

- **Plaintext default.** `Wallet::new` opens or creates a plaintext file unless the caller
  uses `open_encrypted`. The desktop UI never prompts for a password and never calls
  `set_password`, so the on-disk `~/.miraset/wallet.json` is **unencrypted by default**.
  The encryption code path exists and is well-tested (`lib.rs:498-591`) but is opt-in.
- **Secret export round-trip.** `export_secret` returns the raw secret as a string to the
  WebView, which then displays it inline. There is no clipboard safety wrapper, no expiry,
  and no confirmation modal.
- **Hard-coded service URLs.** `WORKER_URL` and `OLLAMA_URL` are constants in the UI
  (`page.tsx:7-8`); only `RPC_URL` is user-configurable.
- **Event parsing is heuristic.** `Transaction History` (`page.tsx:850-940`) inspects
  object keys to infer event type and renders detail by string-slicing. It special-cases
  several serde tagged-enum key names (`Transferred`, `ChatMessage`, `WorkerRegistered`,
  `JobCreated`, `JobCompleted`) and falls back to `JSON.stringify(...).slice(0, 80)` when
  nothing matches.
- **No retry / backoff on transient RPC errors.** The 10 s balance poller and the 10 s
  connection poller silently swallow errors.
- **`reqwest` client per call.** `check_connections` builds a new `Client` with a 3 s
  timeout per probe (`main.rs:194-207`); other commands use `reqwest::get` / a fresh
  `Client::new()`. No connection pooling, no shared headers, no auth.
- **No signing for `SubmitJob` from the wallet.** `submit_job` (`main.rs:249-280`) POSTs
  the requester address (hex) but does not attach a signature. The node coordinator
  endpoint must therefore authenticate the requester another way (or the call is
  unauthenticated); this should be reconciled with the chain's tx auth model.
- **Single-key keystore model.** An account is a single 32-byte ed25519 secret. There is
  no HD/BIP-style derivation, no mnemonics, no multi-sig, and no hardware-wallet
  integration.
- **No mobile / web build.** The package is desktop-only (Tauri requires a native
  runtime). The Tauri IPC is the only client API; there is no separate JS SDK.
- **`tauri.conf.json` security defaults.** `csp: null` and `bundle.active: false` are
  fine for development and the bespoke `package-zip.mjs` distribution, but should be
  reviewed before public release.

## 8. Summary

The wallet is a deliberately small, well-bounded product: a Next.js UI for humans, a
Tauri shell for OS integration and a Rust signer, and a `miraset-wallet` crate that does
one job (encrypted or plaintext key store) and does it with strong, conventional choices
(ed25519, Argon2id, AES-256-GCM). The chain interaction is brokered through a flat set of
Tauri commands that map 1:1 to the node's HTTP API. The main gaps to close before treating
this as a release-quality wallet are: enforce an encrypted wallet by default, gate secret
export behind a confirmation, and make the worker / Ollama URLs configurable like the RPC
URL already is.
