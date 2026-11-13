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
an in-process key or `transport` for `GuardedNetwork`, `reason` = `stale`, `conflicting` or `below_lock`). A
validator with `--remote-signer` has no `signer` series; that process counts its own refusals.
Refused or closed P2P connections are counted as `miraset_p2p_rejected_total{reason}`
(`handshake_timeout`, `handshake_failed`, `bad_attestation`, `unknown_validator`,
`frame_too_large`, `decrypt_failed`, `slots_full`); a rising `unknown_validator` or
`handshake_failed` means something other than a genesis validator is reaching the P2P port.

For measurements (TODO 8.5.3, `docs/measurement-plan.md`) `/metrics` also exports:

| Series | Meaning |
|---|---|
| `miraset_block_txs`, `miraset_block_receipts`, `miraset_block_payload_bytes` (histograms) | shape of each committed block |
| `miraset_empty_blocks_total` | committed blocks with no transaction or receipt |
| `miraset_commit_interval_seconds` (histogram) | time between consecutive consensus commits on this node (block-sync commits excluded) |
| `miraset_unreadable_blocks_total` | committed blocks whose payload could not be read for metrics (should stay 0) |
| `miraset_view`, `miraset_view_timeouts_total`, `miraset_timeout_certificates_total` | consensus view health |
| `miraset_storage_bytes{file="chain_db"\|"signing_guard"}` | database file sizes, read at scrape (redb file length grows in allocation steps; fit growth over long windows) |
| `miraset_process_resident_bytes`, `_cpu_seconds_total`, `_open_fds`, `_threads`, `_write_bytes_total` | process resources from `/proc/self` (Linux) |
| `miraset_p2p_bytes_total{direction}` | P2P bytes of established-session frames (length prefix + encrypted chunks); handshakes and refused frames are not counted |

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
- Blocks carry the proposer's clock (Unix ms). A validator refuses a live proposal stamped more
  than 10 s ahead of or 60 s behind its own clock (logged as "refusing proposal: block timestamp
  too far"), and any block not later than its parent, so validator clocks must be synchronised
  (NTP) well within that; a validator with a skewed clock only follows through block sync. An epoch can be closed only `epoch_seconds` (3600) of
  block time after it began. Receipt clients first allow the coordinator to charge them
  (`system:allowance`, see `docs/rpc-v1.md`); `loadgen` does this for its run-scoped clients.
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
  also keeps a **lock floor**, the highest HotStuff locked view the validator held when it voted
  (TODO 8.5.2), and refuses any vote that builds on a certificate below it, so a resync does not
  lose the lock (see [Restoring chain data](#restoring-chain-data)). Never delete
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
- P2P connections are authenticated (TODO 8.4.1, `docs/adr/proposal-transport-identity.md`):
  each one starts with a Noise `XX` handshake in which both sides prove their genesis validator
  key with a short-lived attestation (10 min, renewed automatically) of a per-start Noise key;
  the chain ID and genesis hash are bound into the handshake. A host that is not a genesis
  validator cannot send consensus messages, and a message's origin is the proven key. Validator
  clocks must stay within about a minute of each other for attestations (NTP, as for block
  timestamps). Refusals are counted by reason (`TcpNetwork::rejections`). Proposals are still
  unsigned in `hotstuff_rs`, so a compromised validator can still propose as itself only.

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

Restoring an **older** chain database rolls back HotStuff's lock and highest views, which live in
the chain data. The guard still refuses every vote in a view the validator already signed, and
every later vote that builds on a certificate below its lock floor (TODO 8.5.2). A single
restored or resynced validator is pulled forward by its peers and votes again once it sees
current certificates.

**If more than f validators are restored below their locks, finality stops** (owner decision
2026-11-12: fail closed). Their guards refuse to vote on the rolled-back fork, which is what
keeps the chain from finalizing different blocks at heights it already finalized. There is no
override: recover by bringing back newer chain data (a healthy peer's `db backup`, or the copy
taken before the restore), then restart. Restoring a backup taken at the current tip (as after
`db backup` of a stopped node) does not roll anything back.

## Known limitations

- **Block rate:** a leader proposes an empty block at most once per second (heartbeat) and a
  block with work at once; `--min-block-interval-ms N` additionally limits every block to one per
  N ms (owner request 2026-11-13; at 1000 the chain commits at most 100 transactions + 20
  receipts per second). Both are capped at half of `max_view_time` (2 s).
- **Pacemaker epoch length is a genesis parameter (TODO 8.5.2 F2):** `protocol.consensus_epoch_length`,
  default 1, range 1..=10, logged at startup (`epoch_length=`). It is part of the genesis
  commitment, so validators of one chain share it, and a node refuses to start with chain data
  created under a different value; changing it means a new genesis. Adding the field changed
  every genesis hash: chain data and signing guards from before it fail with "genesis_hash
  mismatch", so recreate existing devnets (`docker compose down -v`, guards included). Each timed-out view now needs a timeout
  certificate and there is no timeout backoff, so views close to `max_view_time` (slow links,
  large blocks) can time out repeatedly; the WAN re-measurement in 8.5.3 must include that case.
- **Empty-block heartbeat (TODO 8.5.3 finding 11):** without work, a leader proposes an empty
  block at most once per `empty_block_interval` (default 1 s; `max_view_time` default 2 s; the
  interval used is at most half the view time, 0 disables it). Transactions are proposed at
  once by a leader that holds them, but nodes do not gossip transactions, so one sent to a
  single validator can wait up to (N − 1) × interval (≈ 4 s with 5 validators) while other
  leaders send heartbeat blocks first. Send to several validators where latency matters.
- Each node uses roughly 0.2 CPU cores, mostly the consensus thread producing blocks. Upstream
  `hotstuff_rs` busy loops (event bus, block-sync server) are patched in `lib/hotstuff_rs`.
- The validator set is static; reconfiguration is disabled (TODO 8.3.3).
