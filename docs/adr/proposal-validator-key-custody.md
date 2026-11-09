# Proposal: Validator key custody (TODO 8.3.4)

**Status**: Approved 2026-11-09. Decisions 1–4 are approved. D1–D5 are implemented for one
host (§9–§11); a cross-host signer transport (mTLS) depends on TODO 8.4.1. D6 (rotation) is
deferred. Not a numbered ADR; see `docs/adr/README.md`.
**Date**: 2026-11-09
**Requirements**:
- PRD R12 / AT12: protected keys; no secrets in logs.
- `ARCHITECTURE.md`: a signer with "purpose-bound signing with protected key custody".
- `docs/HotStuff-rs.md` §5.8.2: separate the validator key from the node process; support
  external signers.
- TODO 8.3.4: remove deterministic production fallback keys; specify file permissions and a
  reviewed signer boundary before adding an external signer or KMS.

**Sources checked** (vault, 2026-11-09):
- ADR-0008 (approved): Ed25519 signs transactions and workers; BLS12-381 is for committee
  aggregates on settlement.
- ADR-0016 (Proposed), lines 41, 42, 45 and 85:
  - validator and worker keys are distinct;
  - the master/registry authority is protected separately;
  - key rotation and recovery are open design items.

**Scope**: experimental devnet consensus. BLS committee attestations (ADR-0008) and
master-account custody (ADR-0016) are out of scope.

## 1. What exists today

| Fact | Evidence |
|---|---|
| The validator key is a hex file. It is created mode 0600 with `create_new`; reads refuse group/other access and malformed content; secrets are never printed. | `src/devnet.rs:26-80` (`write_key_file`, `read_key_file`), `test_key_files_fail_closed` |
| `--genesis` mode requires `--key-file`. The `[42;32]` key exists only in the in-memory `demo` path. | `src/main.rs:204`, `src/main.rs:357` |
| The secret lives in process memory as several `SigningKey` copies. | `NodeConfig.signing_key`, the clone for `Configuration::me` (`src/node/config.rs:91`), and the `Keypair` the library shares across modules (`lib/hotstuff_rs/src/replica.rs:264`) |
| `SigningKey` zeroizes on drop and its `Debug` hides the secret. The file-read buffers (`String`, decoded `Vec`) are **not** zeroized. | `ed25519-dalek` 2.2 `zeroize` feature; `read_key_file` |
| The library signs in three places, all over raw bytes through `Keypair::sign`: `PhaseVote` (safety-critical), `TimeoutVote`, and `BlockSyncAdvertisement`. | `hotstuff/messages.rs:202`, `pacemaker/messages.rs:45`, `block_sync/messages.rs:97`, `types/crypto_primitives.rs:33` |
| **Devnet: every container mounts every key.** `devnet-init` writes all `nodeN.key` files and `faucet.key` into the shared `devnet-config` volume, and each node mounts that volume. | `docker-compose.yml`, `docs/devnet.md` |
| The double-signing guard (8.3.2) is per host. Two hosts with copies of one key each have their own guard and can equivocate. | proposal-vote-safety-state §9, R5 |

## 2. Threats

| # | Threat | Today |
|---|---|---|
| K1 | A compromised node (RCE, dependency, debug endpoint) reads the key from disk or memory | The key is in the node's memory and filesystem |
| K2 | One compromised devnet container reads **all** validator keys plus the faucet key | Open: shared mount |
| K3 | Cross-host double signing: failover standby, cloned VM, or a copied key file | Open: the guard is per host |
| K4 | The secret leaks through logs, panic messages, core dumps or swap | Partly mitigated: `Debug` is safe, file buffers are not zeroized, core dumps are not disabled |
| K5 | Key loss or compromise with no rotation | Open: rotation needs a validator-set change (8.3.3, disabled) |
| K6 | A signer that signs arbitrary bytes, so the node can obtain any signature from it | Not applicable yet. Any external signer must avoid this (**purpose-bound** signing). |

## 3. Proposed design

### D1. Purpose-bound signing seam (vendored patch #9)

Replace the library's raw-bytes `Keypair` with a trait. The signer receives a **typed request**,
never arbitrary bytes:

```rust
pub enum SignRequest {
    PhaseVote { chain_id, view, block, phase },
    TimeoutVote { chain_id, view },
    BlockSyncAdvertisement { chain_id, highest_committed_block, .. },
}
pub trait ValidatorSigner: Send + Sync {
    fn public_key(&self) -> VerifyingKey;
    fn sign(&self, request: &SignRequest) -> Result<SignatureBytes, SignerError>;
}
```

The signer serializes the request itself, with the same borsh tuples the library uses today, so
signatures stay byte-identical and the wire format does not change. A refused or failed signature
means the message is not sent. The library's `?` paths and our panic hook (8.3.2 R6) turn signer
faults into a visible halt.

### D2. Slashing protection moves into the signer

`SigningGuard` (8.3.2) is checked **inside** `ValidatorSigner::sign` for `PhaseVote`, before any
signature exists. This is §5.8's "record before the signature is returned". `GuardedNetwork`
stays as a second, independent layer until review (8.5.2) agrees to remove it.

### D3. Two signer implementations

- **`LocalSigner`** (default, devnet): reads the key file in-process, as now, plus the K4 items
  in D4.
- **`RemoteSigner`** client and a `miraset-chain signer` subcommand that runs as **a separate
  process**:
  - the signer process owns the key file **and** the guard; the node has neither;
  - the node sends `SignRequest`s and gets signatures back;
  - one signer serves one validator key and serializes requests, so several nodes (primary and
    standby) can point at it with no cross-host double signing. This closes K3, provided the
    signer process itself is never duplicated;
  - the node stores a pinned signer public key and verifies every returned signature, so a
    wrong-key signer is detected.
- **Transport:** to be decided (decision 2).
- **HSM/KMS backends** (YubiHSM, cloud KMS) plug in behind the signer process later. They are
  out of scope for this item, and their Ed25519 support must be verified per product.

### D4. Key file and process hardening (no protocol change)

- Read key files with `O_NOFOLLOW` and refuse symlinks.
- Check that the file is owned by the process user; mode must stay 0600.
- Zeroize the read buffers (`zeroize` is already a transitive dependency).
- Disable core dumps for the validator and signer processes (`prctl(PR_SET_DUMPABLE, 0)` and
  `RLIMIT_CORE = 0`).
- Keep exactly one in-memory `SigningKey`, owned by the signer. `NodeConfig` holds an
  `Arc<dyn ValidatorSigner>` instead of a `SigningKey`.

### D5. Devnet key isolation

- `devnet-init` writes each validator key to its own `nodeN-key` volume and writes the faucet key
  to a separate volume.
- `devnet-config` holds only `genesis.json`.
- Each node mounts only its own key volume, read-only. `transfer.sh` reads the faucet key through
  a one-shot container.

Option for decision 3: run the devnet with one signer container per validator, so the node
containers hold no key at all.

### D6. Rotation (deferred, documented)

Changing a validator's key changes the validator set, which is a membership change and is
disabled by 8.3.3. This proposal only documents the procedure: emergency removal and re-admission
under the future activation protocol. A replacement key also gets a **fresh** guard file, because
the guard is bound to the validator key.

## 4. Alternatives considered

- **Keep raw-bytes signing and add a remote signer.** Rejected: a compromised node could ask for
  a signature over any bytes (K6). Purpose binding is what makes slashing protection inside the
  signer meaningful.
- **Distributed lock for failover** (etcd/Consul lease, one active signer per key). This is more
  infrastructure and is only safe with fencing tokens checked by the signer. Revisit only if
  several signer processes per key become necessary.
- **Threshold or MPC signing per validator.** Out of proportion for an experimental devnet; its
  real use is the master/registry authority in ADR-0016, which is separate.

## 5. Tests (AT12)

| Level | Test |
|---|---|
| Unit | Each `SignRequest` serializes byte-identically to the library's current message bytes (golden vectors), so signatures are unchanged. The signer refuses a phase vote the guard refuses. Key-file reader rejects symlinks, wrong owner, mode ≠ 0600 and bad length, and zeroizes its buffers. No key bytes appear in any error. |
| Library (vendored) | Patch #9: every signing site goes through `ValidatorSigner`; a signer error stops the message from being sent. |
| Integration | 5-validator cluster with `LocalSigner` behaves as today (T3/T4 from 8.3.2 still pass). The node using `RemoteSigner` never has the key in its config or memory dump path. Two nodes sharing one remote signer produce no equivocation (vote tap as in 8.3.2 T3). A wrong-key signer is detected at startup. |
| E2E | Real binaries: node plus `signer` subcommand; killing the signer halts the node visibly (503, exit 3), and restarting the signer resumes consensus. Docker devnet: each node container can read only its own key (asserted by the fault suite). |

## 6. Cost

- One vendored patch touching three signing sites and `Configuration`. It must be maintained on
  upgrades, like #1–#8.
- With a remote signer, one local round trip per signature (about one phase vote per view),
  which is negligible next to the 1s view time.
- A new subcommand and operator documentation.

## 7. Decisions needed

1. **Approve D1+D2:** purpose-bound `ValidatorSigner` seam (patch #9), with slashing protection
   moved into the signer and `GuardedNetwork` kept as a second layer.
2. **Remote signer transport.** Options:
   - (a) length-prefixed borsh over a **Unix domain socket** (same host; no new dependencies;
     peer-uid check);
   - (b) **mTLS over TCP** (needed for a different host; adds `rustls`);
   - (c) **gRPC**, as named in §5.8 (adds `tonic`/`prost`).

   Proposed: **(a) now**, with the protocol framed so (b) can wrap it later. Cross-host signers
   wait for 8.4.1's transport identity work.
3. **Devnet shape:** D5 key isolation only (proposed), or a separate signer container per
   validator as well.
4. **Order:** D4 + D5 first (small, no protocol change, closes K2/K4), then D1–D3. Proposed:
   **yes**.

## 8. Out of scope / follow-ups

- Key rotation and validator-set activation: 8.3.3.
- Authenticated transport identity, which a cross-host signer would rely on: 8.4.1.
- BLS committee keys (ADR-0008) and master-account custody (ADR-0016).
- `/livez` and `/readyz` probes: 8.4.4.

## 9. Approval record and implementation (2026-11-09)

**Approved by:** project owner, 2026-11-09. Only **decision 4** is approved: implement D4 and D5
first. Decisions 1–3 remain open, so D1–D3 are not started.

| Item | Implementation | Tests |
|---|---|---|
| D4: key file policy | `src/keys.rs`, moved from `src/devnet.rs`, which re-exports it. Opens with `O_NOFOLLOW` and checks the opened handle: regular file, owned by the process user, no group or other bits. Input limited to 256 bytes. Secret buffers held in `Zeroizing`. Write uses `create_new` and 0600, and never follows a symlink. | Unit: `src/keys_tests.rs` covers the policy matrix, symlink read and write, bad mode, length and size, and no secret in errors |
| D4: process hardening | `src/process_hardening.rs` sets `RLIMIT_CORE=0` and, on Linux, `PR_SET_DUMPABLE=0`. It runs at the start of `main` for every subcommand and fails closed. | Integration: `tests/process_hardening.rs`. E2E: `tests/e2e_validator.rs::test_e2e_validator_process_is_hardened` checks the core limit is 0 and that `/proc/<pid>/environ` cannot be read by the same user. |
| D4: one in-memory copy | Done with D1 (§10). The validator key lives only inside its signer. `NodeConfig` and the library hold an `Arc<dyn ValidatorSigner>`. | — |
| D5: devnet key isolation | `devnet-init --keys-dir` writes `node{i}/node{i}.key` and `faucet/faucet.key`, with new key directories at 0700. Compose gives each key its own volume, and each node mounts only its own key read-only. The faucet key goes only to the `faucet` tool (profile `tools`, no network). `transfer.sh` uses that tool. | Unit: `test_init_devnet_keys_dir_puts_each_key_in_its_own_directory`. E2E: the Docker fault suite's "key isolation" step checks each node sees exactly its own key. The full suite passed 2026-11-09. |

**Threat status after D4 and D5:**
- **K2** (one container reads every key): closed for the devnet.
- **K4** (leaks through core dumps, ptrace or `/proc`): mitigated. In-memory copies remain until
  D1.
- **K1, K3, K6**: open until D1–D3.
- **K5** (rotation): deferred (D6).

## 10. Decisions 1 and 3 (2026-11-09)

**Approved by:** project owner, 2026-11-09.
- **Decision 1** is approved: the D1 signer seam and D2 slashing protection inside the signer.
- **Decision 3** is approved as proposed: the devnet uses D5 key isolation only, with no
  separate signer containers. Those would need the remote transport, and decision 2 is still
  open.

| Item | Implementation | Tests |
|---|---|---|
| D1: purpose-bound seam | Vendored patch #9 (`lib/hotstuff_rs/MIRASET_PATCHES.md`): `SignRequest`, `ValidatorSigner`, `LocalKeySigner`, `SignError::{Refused, Failed}`. No raw-bytes signing path is left. `NodeConfig` holds `signer: Arc<dyn ValidatorSigner>` instead of a `SigningKey`, and its `Debug` shows only the public key. | Library: `src/types/signer_tests.rs` (golden bytes, byte-identical signatures). Integration: refusal means no vote and no halt (`tests/vote_safety.rs`); failure halts (`tests/signer_fault_halt.rs`). |
| D2: guard inside the signer | `src/consensus/signer.rs` `GuardedSigner`. A phase vote is authorized by `SigningGuard::authorize_record` before it is signed: stale or conflicting votes are refused, and a guard fault is a signer failure. Timeout votes and advertisements are signed without the guard. The constructor refuses a guard bound to another key. `GuardedNetwork` shares the guard as the second layer and sees each signed vote as an identical re-send. | Unit: `src/consensus/signer_tests.rs`. Integration: the 5-validator cluster harness (`tests/vote_safety_cluster.rs`) now runs `GuardedSigner`, and refusals are counted across both layers. E2E: real binary (`tests/e2e_validator.rs`) and the Docker fault suite, both passed 2026-11-09. |

**Threat status after D1 and D2:**
- **K6** (signing arbitrary bytes): closed.
- **K4** (leaks through core dumps or memory): one key copy, non-dumpable process, zeroized file
  buffers.
- **K1** (the key is in the node process) and **K3** (cross-host double signing): open until D3,
  the remote signer, which waits for decision 2 (transport).

## 11. Decision 2: gRPC, and the D3 remote signer (2026-11-09)

**Approved by:** project owner, 2026-11-09. Decision 2 is option **(c), gRPC**, built on
`tonic`/`prost`. The protobuf is compiled in `build.rs` by the pure-Rust `protox`, so no system
`protoc` is needed locally or in Docker.

**Transport:**
- **Now:** gRPC over a **Unix domain socket**. The socket is created with mode 0600, and each
  connection's peer uid (`SO_PEERCRED`) must equal the signer's uid.
- **Later:** gRPC over TCP with mTLS for a signer on another host. It waits for 8.4.1
  (authenticated transport identity) and is not built yet.

| Item | Implementation | Tests |
|---|---|---|
| Wire contract | `proto/signer.proto` (`miraset.signer.v1.ValidatorSigner`): `GetPublicKey` and `Sign`, with typed `PhaseVote`/`TimeoutVote`/`AdvertiseBlock` requests; the response is a signature or an explicit `refused`, and faults are gRPC errors. `src/signer/convert.rs` validates every field (32-byte hash, known phase, request present). | Unit: round trip with identical signing bytes; malformed requests are rejected |
| Signer process | `miraset-chain signer --genesis --key-file --signing-guard --socket [--init-signing-guard]` (`src/signer/process.rs`, `server.rs`). It owns key and guard through `GuardedSigner` and never creates a guard implicitly. A fault halts it: it signs nothing more, exits with code 3, and **stops even with connected clients after a 2s grace**. Without that bound, tonic's graceful shutdown would wait for every client to disconnect. A stale socket file is replaced, and anything else at that path is refused. | Unit: peer-uid rule, socket replacement and mode. Integration: `tests/remote_signer.rs`. E2E: the signer refuses to start without a guard. |
| Remote client | `src/signer/client.rs` `RemoteSigner` implements `ValidatorSigner`: <ul><li>checks the chain id and an optional `--validator-pubkey` pin at connect;</li><li>verifies **every** returned signature against the signer's key over the exact signing bytes;</li><li>sets a 2s deadline per request;</li><li>retries connecting for 10s at startup only.</li></ul> A failure halts the validator (exit 3, `/health` 503). | Unit: identity checks, signature verification. Integration: remote signatures are identical to local ones; refusals cross the wire; **4 concurrent clients never get two votes signed for one view** (K3 on one host); a fault halts the server; a wrong pin or chain is rejected; a validator with a remote signer finalizes. E2E: real `signer` + `--remote-signer` validator finalizes, the validator is given no key path, and the guard is recorded in the signer. **SIGKILLing the signer** gives the validator `/health` 503 and exit 3, and restarting both resumes the chain. A signer for another key is refused (exit 1). |
| Node-side layer | With `--remote-signer` the node still opens its own `--signing-guard` for `GuardedNetwork`, as the second layer. | (existing 8.3.2 tests) |

**Threat status:**
- **K1** (the key in the node process): closed when the validator runs with `--remote-signer`.
  The node process has no key, and the signer runs as its own hardened process.
- **K3** (double signing): closed **on one host**. Any number of validator processes can share
  one signer, which serializes requests and keeps one guard. Cross-host sharing needs the mTLS
  transport (8.4.1).
- **K5** (rotation): still deferred (D6, 8.3.3).
- **Devnet:** decision 3 keeps the in-process `GuardedSigner`, so no signer containers.

