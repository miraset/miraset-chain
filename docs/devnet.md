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
scripts/devnet/transfer.sh bob 1000   # signed faucet transfer via node0
scripts/devnet/fault_suite.sh         # crash / pause / partition scenarios (~1 min)
docker compose down -v                # stop and delete the chain (keys, genesis, databases)
```

RPC for `nodeN` is published on `127.0.0.1:854{5+N}` (node0 = 8545 … node4 = 8549). Useful
endpoints: `/health`, `/block/latest` (hash + height), `/block/height/{h}`, `/account/{addr}`,
`/tx/submit`, `/mempool/status`, `/events/blocks` (SSE).

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

## Layout and security notes

- `devnet-init` writes `genesis.json`, `node{i}.key`, and `faucet.key` (mode 0600, uid 10001) to
  the `devnet-config` volume. It is a no-op if genesis already exists.
- Every node mounts the whole config volume read-only, so each container can read every key. This
  is acceptable only for a local devnet; real deployments need per-validator key custody (TODO 8.3.4).
- Nodes refuse key files readable by group/others, keys that are not genesis validators, and a
  database created from a different genesis (accounts or validator set).
- Each validator keeps a vote **signing guard** (`signing-guard.redb`) on its own `nodeN-guard`
  volume, separate from chain data `nodeN-data` (TODO 8.3.2). Deleting only `nodeN-data` is a
  safe resync: the guard stops the node re-signing views it already voted in. Never delete
  `nodeN-guard` for a validator that has signed; a node with chain data but no guard refuses to
  start unless `--init-signing-guard` is passed.
- **Upgrading an existing devnet:** older devnets have chain data but no guard volume, so the
  validators refuse to start. If you do not need the old chain, use `docker compose down -v`.
  Otherwise start each validator once with `--init-signing-guard` added to its command. This is
  safe only because no guard existed before, so nothing could be double-signed against it.
- A consensus safety halt (guard fault, chain write failure, or a panic on any `hs-*` consensus
  thread) makes `/health` return 503
  `{"status":"halted","reason":…}` and the process exit with code 3 after 5s; compose restarts it
  (`restart: on-failure`). Stale or conflicting votes are dropped and logged, not halted.
- P2P identity is a self-declared key checked against the genesis peer table, not authenticated
  (TODO 8.4.1). The devnet network must stay private.

## Known limitations

- Each node uses roughly 0.2 CPU cores, mostly the consensus thread producing blocks. Upstream
  `hotstuff_rs` busy loops (event bus, block-sync server) are patched in `lib/hotstuff_rs`.
- The validator set is static; reconfiguration is disabled (TODO 8.3.3).
