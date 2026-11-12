# Local Docker devnet (experimental)

A 5-validator HotStuff devnet for consensus and fault testing. It is **experimental**: ADR-0016
is Proposed, so this is not production settlement and must not be exposed publicly.

## Why 5 validators

`hotstuff_rs` v0.4.0 rotates leaders strictly by view, sends votes to the leader of `view + 1`,
and commits only on a 3-chain of consecutive-view QCs. A commit therefore needs four consecutive
live leaders. With 4 equal-power validators and one down that never happens, so finality halts.
With 5, the remaining 4-of-5 quorum keeps committing (one window per 5 views).

## Run

```bash
docker compose up -d --build          # devnet-init generates keys + genesis, then node0..node4 start
python3 scripts/devnet/check_consensus.py --progress-window 20 --max-lag 5
scripts/devnet/transfer.sh bob 1000   # faucet transfer, signed in the one-shot `faucet` tool container
scripts/devnet/fault_suite.sh         # crash / pause / partition scenarios (~1 min)
docker compose down -v                # stop and delete the chain (keys, genesis, databases)
```

RPC for `nodeN` is published on `127.0.0.1:854{5+N}` (node0 = 8545 … node4 = 8549). Useful
endpoints: `/v1/health`, `/v1/blocks/latest` (hash + height), `/v1/blocks/{h}`, `/v1/accounts/{addr}`,
`/v1/transactions`, `/v1/mempool`, `/v1/events/blocks` (SSE). The full client contract (status codes,
freshness labels, API keys, rate limits) is [`docs/rpc-v1.md`](rpc-v1.md); the devnet runs
without an API key file, so submissions are open.

Operations endpoints (TODO 8.4.4):

| Endpoint | 200 when | 503 body `status` |
|---|---|---|
| `/livez` | the process serves and consensus has not halted | `halted` |
| `/readyz` | not halted, not draining, no block-sync session open, and a block was committed through consensus within `--ready-max-commit-age-secs` (default 30) | `halted`, `draining`, `syncing`, `no_commit_yet`, `commit_stale` |
| `/metrics` | always; Prometheus text format with a fixed metric set (`miraset_*`) | — |

`/readyz` measures whether this node is finalizing, so every node of a set that cannot reach a
quorum reports not ready. Blocks committed through block sync do not count: a lagging node
reports `syncing` during a sync session and `commit_stale` (or `no_commit_yet`) between
sessions (each session discards the earlier consensus commit), and turns ready only after it
commits through consensus, which needs the blocks up to the tip. Such a commit can finalize an
ancestor a few heights below the tip. If the highest certificate does not advance for
`block_sync_trigger_timeout` (3 s), for example under heavy netem loss, the node starts a sync
session, so `/readyz` can alternate between `ready` and `syncing` on a slow chain that is
still finalizing. The compose
healthcheck uses `/livez`: a node without quorum or catching up is alive, not unhealthy.

Besides commits, admission, mempool and transport drops, `/metrics` exports block sync
(`miraset_syncing`, `miraset_sync_sessions_total`, `miraset_blocks_synced_total`) and phase votes
refused by the signing guard (`miraset_vote_refused_total{layer, reason}`, `layer` = `signer` for
an in-process key or `transport` for `GuardedNetwork`, `reason` = `stale` or `conflicting`). A
validator with `--remote-signer` has no `signer` series; that process counts its own refusals.

Logs go to stderr: text by default, one JSON object per line with `--log-format json`; `RUST_LOG`
sets the filter (default `info`). On SIGTERM a validator stops admission (`/v1/transactions` and
`/v1/receipts` return 503), reports `draining` for `--shutdown-drain-ms` (default 2000), then
stops consensus and logs how many mempool items were discarded; the mempool is not durable.
A second SIGTERM/SIGINT ends the drain early, and a safety halt during the drain still exits with
code 3. Safety events (`SAFETY HALT`, refused or dropped votes) use the `miraset_safety` target,
which stays enabled at `warn` whatever `RUST_LOG` says; an invalid `RUST_LOG` is reported and
falls back to `info`.

Changes for operators: the startup banner is now a log line on **stderr** (previously stdout),
and admission errors read `already exists in mempool` (400) or `node is shutting down…` (503).
`/metrics` and `/readyz` are served on the same unauthenticated RPC port as everything else; they
expose only fixed, non-secret series, but keep the port private (public RPC hardening is 8.4.3).

## What the fault suite asserts

| Scenario | Fault | While faulted | After recovery |
|---|---|---|---|
| Crash | `docker compose stop node4` | other 4 keep committing, agree at every height | node4 restarts from its own volume and catches up |
| Hang | `docker compose pause node3` | same | node3 catches up |
| Partition | `docker network disconnect` node2 | same | node2 rejoins (new IP) and catches up |
| SIGKILL | `docker compose kill -s SIGKILL node1` | same | node1 restarts with its guard record and catches up |
| Resync | delete node1's chain-data volume, keep `node1-guard` | same | node1 resyncs from genesis and catches up; no `SAFETY HALT` in any log |

`check_consensus.py` exits non-zero on conflicting committed hashes at any height (safety), no
progress within the window (liveness), or a node more than `--max-lag` blocks behind (catch-up).

## Measuring throughput, finality and stability (TODO 8.5.3)

`scripts/devnet/bench.sh` sweeps offered rates with `miraset-chain loadgen` (the `loadgen`
compose service, profile `tools`), `scripts/devnet/netem.sh` emulates WAN latency and loss on
validator egress, and `scripts/devnet/soak.sh` runs a constant-rate soak while sampling memory,
file descriptors, data size and height. Workload, procedure, measured baseline and findings are
in [`docs/performance-baseline.md`](performance-baseline.md). No target is approved.

## Layout and security notes

- `devnet-init` writes `genesis.json` to the `devnet-config` volume. Each key goes to **its own
  volume**: `node{i}-key` holds `node{i}.key` and `faucet-key` holds `faucet.key` (mode 0600,
  uid 10001, directories 0700). It is a no-op if genesis already exists.
- Each validator container mounts only its own key volume, read-only, at
  `/var/lib/miraset-key`. The faucet key is visible only to the one-shot `faucet` tool container
  (profile `tools`, no network) used by `transfer.sh` and the `loadgen` tool. The fault suite
  checks that each node sees exactly one key file (TODO 8.3.4, key custody D5).
- Genesis also lists one devnet **coordinator** key (`coordinator-key` volume; co-signs compute
  receipts) and one **governance** key (`governance-key`; slashes worker bonds and closes epochs,
  which mints the epoch reward), plus `price_per_token` and `epoch_reward_budget` (TODO 8.5.1,
  `GET /v1/params`). No validator mounts them; `loadgen` mounts the coordinator key read-only.
  A genesis without them settles no receipt and closes no epoch.
- Key files must be regular files (symlinks are refused), owned by the process user, with no
  group or other access. Nodes also refuse keys that are not genesis validators, and a database
  created from a different genesis (accounts, protocol parameters or validator set).
- Every `miraset-chain` process disables core dumps and marks itself non-dumpable on Linux, so
  other processes of the same user cannot read its memory or environment (key custody D4).
- Outside the devnet, the key can live in a separate signer process instead: run
  `miraset-chain signer --genesis … --key-file … --signing-guard … --socket …` (the first start
  needs `--init-signing-guard`), then start the validator with `--remote-signer <socket>` and no
  `--key-file`. Both must run as the same user.
- Each validator signs through `GuardedSigner`, the only holder of its key. It checks the signing
  guard before it signs a phase vote, and it never signs arbitrary bytes (key custody D1+D2).
  The key is still inside the validator process; a separate signer process (D3) awaits the
  transport decision.
- Each validator keeps a vote **signing guard** (`signing-guard.redb`) on its own `nodeN-guard`
  volume, separate from chain data `nodeN-data` (TODO 8.3.2). Deleting only `nodeN-data`
  resyncs the node, and the guard stops it re-signing any view it already voted in. The guard
  does **not** keep HotStuff's lock (locked certificate), which lives in the chain data: until
  the node is past the views it voted in before, it may vote without its old lock, so treat it
  as one of the f faulty validators for that time and resync at most one validator at once
  (see [Restoring chain data](#restoring-chain-data)). Never delete
  `nodeN-guard` for a validator that has signed; a node with chain data but no guard refuses to
  start unless `--init-signing-guard` is passed.
- **Upgrading an existing devnet:** recreate it with `docker compose down -v`. Older devnets
  keep their keys in `devnet-config`, which the nodes no longer read, and have no guard
  volumes.
  Since TODO 8.5.1 genesis also holds protocol parameters and receipts carry a worker signature
  (the genesis hash, stored epochs and receipt encoding changed): a devnet created before it
  fails closed on start ("genesis_hash mismatch") and must be recreated; an old `genesis.json`
  without a `protocol` section loads but has no coordinator or governance key.
- A consensus safety halt (guard fault, chain write failure, or a panic on any `hs-*` consensus
  thread) makes `/v1/health` return 503
  `{"status":"halted","reason":…}` and the process exit with code 3 after 5s; compose restarts it
  (`restart: on-failure`). Stale or conflicting votes are dropped and logged, not halted.
- P2P identity is a self-declared key checked against the genesis peer table, not authenticated
  (TODO 8.4.1, design in `docs/adr/proposal-transport-identity.md`). Proposals are unsigned in
  `hotstuff_rs`, so any host that reaches a P2P port can impersonate the leader. The devnet
  network must stay private.

## Starting and maintaining a validator (TODO 8.4.5)

Validators start only with the `start` subcommand (`miraset-chain start --genesis … --key-file …`
or `--remote-signer …`); the earlier form without a subcommand is rejected, and running the
binary with no arguments prints help. The in-memory demo is `miraset-chain demo`.

Offline maintenance works on a **stopped** validator's `--data-dir` and prints JSON on stdout;
errors go to stderr as `error: …` with exit code 4 (database in use), 5 (needs crash recovery,
or verification failed) or 1 (anything else).

| Command | Does | Writes |
|---|---|---|
| `db inspect` | chain ID, genesis commitment, schema version, commit sequence, committed height and block | nothing (read-only open) |
| `db verify [--genesis FILE]` | recovery metadata (and the genesis file's chain ID and commitment); every committed height: the stored hash recomputes, height and parent link, the justify is for this chain with increasing views, the payload is canonical and matches its data hash; nothing indexed above the tip. A file that is unreadable or damaged fails with exit 5 or 1 instead of crashing. Does not re-verify quorum-certificate signatures (checked on insert) or redb page checksums (see `compact`) | nothing (read-only open) |
| `db compact` | redb page-checksum check (repairs if needed; `repaired` in the report), then reclaims free pages | the database file, without changing records |
| `db backup --out FILE` | logical copy of one read-only snapshot (every table) into a new file (mode 0600), closed cleanly, then `verify`d against the source's chain identity and tip | only the new file; never overwrites; removed again if anything fails |

`inspect` and `verify` refuse a database that needs crash recovery (a validator that was killed):
start the validator once, or run `db compact`, which repairs it. Every command refuses a database
that a running validator holds and never creates or truncates one; this relies on the filesystem
honouring `flock` (local disks and local Docker volumes do, some network filesystems do not).
Inside Docker, run them on a stopped node's volumes, for example
`docker compose run --rm --no-deps node1 db verify --data-dir /var/lib/miraset --genesis /devnet/genesis.json`.

### Restoring chain data

**The signing guard is never backed up.** Restoring an older guard would roll back slashing
protection and allow double signing; always keep the current guard.

Restoring an **older** chain database also rolls back state the guard does not cover: HotStuff's
lock (locked certificate) and highest views live in the chain data. The guard still refuses every
vote in a view the validator already signed, but in later views the restored validator may vote
without its old lock. Until it is past the views it voted in before the restore, count it as one
of the f faulty validators: restore (or resync) at most one validator at a time, and only while
the rest of the set is healthy. Restoring a backup taken at the current tip (as after
`db backup` of a stopped node) does not roll anything back.

## Known limitations

- Each node uses roughly 0.2 CPU cores, mostly the consensus thread producing blocks. Upstream
  `hotstuff_rs` busy loops (event bus, block-sync server) are patched in `lib/hotstuff_rs`.
- The validator set is static; reconfiguration is disabled (TODO 8.3.3).
