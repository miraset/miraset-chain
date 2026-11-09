# Proposal: Authenticated validator transport (TODO 8.4.1)

**Status**: Proposed 2026-11-09, awaiting approval (§8). Not a numbered ADR; see
`docs/adr/README.md`.
**Date**: 2026-11-09
**Requirements**:
- PRD R09 / AT09: authenticate peers and messages; bound bytes, queues, pending work and
  verification; reject malformed, unauthorized, replayed or stale traffic within configured limits.
- `ARCHITECTURE.md` R09 row: frames authenticated with Ed25519 and bound to `chain_id` and the
  sender `VerifyingKey`; this proposal refines *how* (§5).
- `docs/HotStuff-rs.md` §5.1: a Noise `XX` handshake that binds the peer identity to the
  validator public key.
- TODO 8.4.1: decide whether static TCP is enough or libp2p/QUIC is needed; no discovery or
  gossip by assumption.
- Follow-up dependency: a cross-host remote signer (proposal-validator-key-custody §11).

**Sources checked** (vault, 2026-11-09):
- ADR-0016 (Proposed), lines 26, 32, 41: one MIRASET-operated, **permissioned** network;
  validators are admitted by the operator, each with a unique validator key.
- ADR-0016 line 59 specifies gRPC/TLS for the **worker job** transport, not for consensus
  peers. It is out of scope here.
- ADR-0008 (approved): Ed25519 for validator keys. This proposal adds an X25519 key only for
  transport key agreement. It never signs consensus data.

**Scope**: the validator-to-validator consensus transport (`src/network/tcp.rs`). Public RPC
(8.4.3), state sync trust (8.4.2) and validator-set changes (8.3.3) are out of scope.

## 1. What exists today

| Fact | Evidence |
|---|---|
| Each frame carries a **self-declared** 32-byte sender key. The receiver only checks that the key is in the peer table. | `src/network/tcp.rs:8-10`, `FrameReader::dispatch` |
| No encryption and no integrity protection beyond TCP. | `src/network/tcp.rs` |
| **Proposals and nudges are unsigned.** The library accepts them if the transport `origin` is the current leader. | `lib/hotstuff_rs/src/hotstuff/implementation.rs:329-345` (`is_proposer(origin, …)`), `hotstuff/messages.rs:116` |
| Votes, timeout votes and sync advertisements are signed by the validator key and verified against `origin`. | `hotstuff/sequence_flow.rs:181-183`, patch #9 `SignRequest` |
| Bounds are compile-time constants: 16 MiB frames, 64 inbound connections, queues of 4096 inbound and 1024 per peer. Any host can take inbound slots before sending a byte. | `src/network/tcp.rs:27-31` |
| Outbound and inbound connections are separate, one-directional TCP streams per peer pair. | `PeerWorker`, `Acceptor` |

## 2. Threats

| # | Threat | Today |
|---|---|---|
| N1 | **Leader spoofing**: anyone who reaches the P2P port sends a `Proposal` claiming the leader key. Honest validators vote for a block the attacker chose, and the real proposal is then ignored (`has_one_leader_proposed`). | **Open.** It needs only network reach, no key. |
| N2 | Forged `origin` on other messages (NewView, block sync requests and responses) to misattribute or poison them | Open. Signed payloads limit the damage; unsigned ones do not. |
| N3 | Connection-slot exhaustion: 64 unauthenticated idle connections block real peers for up to 10 s each, repeatedly | Open |
| N4 | Oversized or expensive frames: a 16 MiB frame from an unauthenticated sender is allocated before any check | Open |
| N5 | Replay or reordering of captured frames | Partly mitigated: signed votes are view-bound and the guard drops stale votes. Unsigned messages can be replayed. |
| N6 | Passive observation of consensus traffic (votes, transactions in blocks) | Open. Low impact (the data becomes public on finality) but it reveals timing and topology. |

N1 is the main reason for this item. Until it is fixed, the experimental network must not be
reachable from untrusted networks. The devnet runs on a private Docker network.

## 3. Proposed design

### T1. Noise `XX` handshake per connection

Every TCP connection starts with `Noise_XX_25519_ChaChaPoly_SHA256` (crate `snow`). The
prologue binds the session to the network:

```
prologue = "MIRASET:P2P:v1" || chain_id (u64 BE) || genesis_hash (32 bytes)
```

A mismatch in chain, genesis or protocol version fails the handshake before any consensus
message is read. A handshake deadline (default 2 s) bounds pre-authentication time.

### T2. Bind the Noise static key to the validator key (attestation)

The Noise static key is X25519, but validators are identified by Ed25519 keys. Each node proves
the link with a **transport attestation** in its handshake payload:

```
SignRequest::TransportIdentity { chain_id, noise_static_pubkey: [u8; 32] }
```

- This is a new purpose in the `ValidatorSigner` seam (vendored patch #9 extension). Its signing
  bytes are domain-separated (`"MIRASET:P2P-ID:v1"`), so they cannot collide with votes, timeouts
  or advertisements. The signer still never signs raw bytes (custody K6).
- The receiver checks: the attestation signature is valid for a key in the configured validator
  set, `chain_id` matches, and `noise_static_pubkey` equals the static key Noise authenticated.
- For a **remote signer**, `proto/signer.proto` gains this request type. The signer applies no
  slashing rule to it, because an attestation cannot cause equivocation.

### T3. Ephemeral static key per process start

The node generates a fresh X25519 static key at every start and asks the signer for one
attestation. There is **no transport key on disk**, so there is no new file to protect, rotate
or isolate. A stolen attestation is useless without the matching X25519 secret, which exists
only in the memory of the process that requested it.

Consequence: a compromised running node can impersonate itself, which it can do anyway. It
cannot impersonate another validator.

### T4. `origin` comes from the authenticated session

- The self-declared sender field is removed from the frame. `recv()` returns the validator key
  proven in T2 for that connection.
- Outbound connections verify that the responder is the validator we dialled. A DNS or IP
  redirect to another validator or a stranger fails.
- At most **one authenticated inbound connection per validator**. A new one replaces the old one,
  so a restarted peer reconnects at once.

### T5. Transport framing and replay

- Noise messages are at most 65 535 bytes. A consensus frame is sent as
  `u32 length || encrypted chunks`, and the length is checked against the configured maximum
  **before** allocating.
- Noise transport nonces are implicit counters: replaying, dropping or reordering an encrypted
  chunk breaks decryption and closes the connection (N5 inside a session). Fresh ephemeral keys
  per handshake stop replay across sessions.
- Replaying a *message* (for example re-sending an old proposal) needs a validator's live session,
  so it reduces to a compromised validator. HotStuff view rules and the signing guard already
  cover that case.

### T6. Configurable limits and observable rejections

`TransportLimits` in `NodeConfig`, with CLI flags and validated defaults:

| Limit | Default | Applies |
|---|---|---|
| `max_frame_bytes` | 16 MiB | after authentication; pre-auth handshake messages are capped at 1 KiB |
| `max_pending_handshakes` | 16 | unauthenticated connections; replaces today's single cap of 64 |
| `handshake_timeout` | 2 s | |
| `max_inbound_per_peer` | 1 | authenticated |
| `inbound_queue` / `peer_queue` | 4096 / 1024 | as today |
| `inbound_idle_timeout` | 10 s | as today |

Every rejection increments a labelled counter with bounded cardinality (`reason` from a
fixed enum: `handshake_timeout`, `bad_prologue`, `bad_attestation`, `unknown_validator`,
`frame_too_large`, `decrypt_failed`, `queue_full`, `slots_full`). The counters are exposed
through the existing `dropped_messages`-style accessors now and through metrics in 8.4.4. Logs
name the reason and the peer address, never key material.

### T7. Transport choice: static TCP + Noise, no libp2p/QUIC

ADR-0016 describes a small, operator-admitted validator set with configured addresses. Static
TCP covers that:
- no discovery or gossip (TODO 8.4.1 forbids adding them by assumption);
- `snow` is one small, widely used dependency, against libp2p's large stack, which would replace
  the transport just hardened in 8.3.1.

Revisit libp2p/QUIC only if an approved requirement adds non-validator peers, NAT traversal or
gossip.

## 4. Alternatives considered

- **Sign every frame with the validator key** (the literal `ARCHITECTURE.md` wording). Rejected:
  - it needs a signer call per message (hundreds per second through a remote signer);
  - it needs a raw-frame signing purpose, which weakens custody K6;
  - it gives no encryption and still needs a separate replay scheme.
  One attestation per connection gives the same origin authenticity.
- **Mutual TLS (`rustls`)**. Equivalent security. However:
  - with the validator key as the certificate key, the signer would have to sign TLS transcripts,
    which are raw bytes (K6);
  - so a separate TLS key plus the same attestation would still be needed;
  - certificates and custom verifiers add complexity without benefit for a pinned, static peer
    set.
  `rustls` stays the planned choice for public RPC (8.4.3) and the worker transport (ADR-0016).
- **Custom handshake** (Ed25519 challenge + X25519 + AEAD). Rejected: hand-rolled key agreement
  repeats what Noise specifies and analyses.
- **Noise `IK`** (one round trip fewer). It needs the responder's static key in advance. T3 makes
  static keys change on every restart, so `XX` fits. The extra round trip only costs at connect
  time.

## 5. Change to `ARCHITECTURE.md` (on approval)

Replace the R09 row's "Ed25519 envelope signatures" with: "Noise `XX` sessions whose static key is
attested by the validator key (purpose-bound `TransportIdentity`), bound to `chain_id` and
genesis; origin is the authenticated peer; configurable pre- and post-authentication limits."

## 6. Tests (AT09)

| Level | Test |
|---|---|
| Unit | Attestation signing bytes are a golden vector and differ from every other `SignRequest`. Attestation verification rejects a wrong chain, an unknown validator, a mismatched Noise key and a bad signature. Frame reader rejects an oversized length before allocating. Limits validation rejects zero and absurd values. |
| Library (vendored) | `TransportIdentity` goes through `ValidatorSigner`; `GuardedSigner` passes it without touching the guard. |
| Integration | Two-node and cluster tests over the new transport (existing `tests/tcp_network.rs`, `tcp_cluster.rs`, `vote_safety_cluster.rs` unchanged in behaviour). Adversarial `tests/transport_auth.rs`: **N1 spoofed-leader proposal over raw TCP is rejected and never reaches the library**; a non-validator with a valid Noise key fails; a replayed attestation with another Noise key fails; wrong chain or genesis fails in the handshake; pending-handshake flood does not block a real peer; oversized, truncated and garbage frames close only that connection; a tampered or replayed ciphertext chunk closes the connection; each case increments its reason counter. |
| Remote signer | `TransportIdentity` over gRPC, including a node that refuses an attestation for a different key from a pinned signer. |
| E2E | Real binaries and the Docker fault suite still pass; a script that injects a raw spoofed proposal against a devnet node shows the rejection counter rising and no vote for the injected block. |

## 7. Cost (estimates)

| Part | Estimate |
|---|---|
| Patch #9 extension (`TransportIdentity`), signer proto and `GuardedSigner` | 3–4 h |
| Noise session layer, chunked framing, attestation exchange | 8–12 h |
| `TransportLimits`, pre/post-auth slots, reason counters, CLI flags | 4–6 h |
| Tests (unit, integration, adversarial, remote signer, e2e, Docker suite) | 8–12 h |
| Docs (`ARCHITECTURE.md`, devnet, TODO evidence) and review fixes | 2–3 h |
| **Total** | **about 25–37 h (about 1 working week)** |

Runtime cost: one handshake and one signer call per connection (and per reconnect); ChaCha20-
Poly1305 on consensus traffic, negligible next to 350 ms–1 s views. One new dependency (`snow`,
plus its pure-Rust crypto backends), subject to a licence and maintenance review.

## 8. Decisions needed

1. **Approve T1–T5**: Noise `XX` with a validator-attested, per-start static key, replacing the
   self-declared sender. This includes the `TransportIdentity` signer purpose (patch #9
   extension, signer proto change).
2. **Approve T7**: stay on static TCP + Noise and do not adopt libp2p/QUIC now.
3. **Approve the new dependency `snow`** (or name a preferred alternative).
4. **Order**: implement this before the cross-host remote signer. Proposed: the cross-host signer
   then reuses the same Noise session layer instead of mTLS. That would amend
   proposal-validator-key-custody §11 "open: mTLS".

## 9. Out of scope / follow-ups

- Public RPC authentication and rate limits: 8.4.3.
- Authenticated sync proofs (what a valid peer sends may still be wrong): 8.4.2.
- Validator-set changes and their effect on the peer table: 8.3.3.
- Metrics export and readiness: 8.4.4.
- Independent review of the handshake binding: 8.5.2.
