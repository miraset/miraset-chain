# Node RPC contract, version 1 (TODO 8.4.3)

**Status:** implemented on the experimental devnet node. Decisions (2026-11-11, project owner):
REST over HTTP under `/v1` only; submissions require a valid signed payload **and** a bearer
API key when the node has a key file; reads are open; per-client token-bucket rate limits.
ADR-0016 (Proposed) lists public RPC authentication and rate limits as open decisions; this
document records the engineering contract chosen for the devnet, not a production approval.

## Conventions

- JSON bodies. `/v1` handlers answer with the envelope
  `{"success": bool, "data": ..., "error": "...", "meta": {...}}` (absent fields omitted),
  except `/v1/health`, which returns its body directly. Responses produced before a handler
  runs are not enveloped: `413` and `408` (empty or plain body), and request-parsing errors
  from the framework (a body that is not valid JSON for the type: `400`/`415`/`422`; a
  non-numeric `{height}` or `{id}`: `400`), all with a plain-text body.
- **Admission is not finality.** A `202` from a submission means the item is pending in *this
  node's* mempool. It is final only once a committed block includes it.
- **Each node serves its own committed state.** Reads never show pending (mempool) effects.
- Nodes do **not** share mempools: an item is proposed only when the node that admitted it
  leads a view, and a sender's pending transactions must all go to one node (see Admission).
- The RPC is plain HTTP. Put TLS in front of it whenever tokens or traffic cross a network.

## Endpoints

| Method and path | Purpose | Success |
|---|---|---|
| `POST /v1/transactions` | submit a signed `Transaction` | `202 {status: "pending", id: "<sender>:<nonce>"}` |
| `POST /v1/receipts` | submit a `ComputeReceipt` co-signed by a genesis coordinator and the worker | `202 {status: "pending", id: "<task_id>"}` |
| `GET /v1/accounts/{address}` | committed balance and nonce | `200`, `404` if absent |
| `GET /v1/workers/{id}` | committed worker record | `200`, `404` |
| `GET /v1/epochs/current`, `/v1/epochs/{id}` | committed epoch | `200`, `404` |
| `GET /v1/tasks/{task_id}` | `{task_id, settled}`: whether a receipt for the task committed; unknown tasks are `settled: false`, which on a stale node can be a false negative | `200` |
| `GET /v1/blocks/latest` | `{highest_committed_block, height}` from one snapshot | `200` |
| `GET /v1/blocks/{height}` | `{height, hash}` of a committed block | `200`, `404` above the tip |
| `GET /v1/mempool` | `{pending_txs, pending_receipts}` on this node | `200` |
| `GET /v1/params` | genesis protocol parameters: `coordinators`, `governance` (hex keys), `price_per_token`, `epoch_reward_budget`, `epoch_seconds`, `consensus_epoch_length` | `200` |
| `GET /v1/allowances/{client}/{coordinator}` | `{client, coordinator, remaining}`: what the coordinator may still charge the client through receipts | `200` |
| `GET /v1/events/blocks` | Server-Sent Events, see below | `200 text/event-stream` |
| `GET /v1/health` | `{status: ok|halted, chain_id, reason?}` (not enveloped) | `200`, `503` when halted |

Operational endpoints are unversioned and outside this contract: `/livez`, `/readyz`
(readiness, see `docs/devnet.md`), `/metrics` (Prometheus). They are public on the RPC port;
restrict them at the proxy if that matters.

## Freshness of reads

Every committed-state read (`accounts`, `workers`, `epochs`, `tasks`, `blocks`, `params`), found or not,
carries `meta`:

| Field | Meaning |
|---|---|
| `committed_height` | highest committed height in this node's store when the read started; the data is at least this new |
| `last_commit_age_secs` | seconds since this node last committed a block **through consensus**; absent since the process started or since a block-sync session began, until the next such commit |
| `stale` | `true` when the node may be behind the chain |
| `stale_reason` | `halted` (safety halt); `syncing` (block-sync session open); `no_commit_yet` (no consensus commit since start or since the last sync session: the node cannot yet tell it is at the tip, even if it has committed state); `commit_stale` (no consensus commit within `--ready-max-commit-age-secs`, default 30) |

Both freshness fields use the same clock and are read once. Block-sync commits never make a
node fresh: only taking part in consensus shows that it is at the tip.

A stale node keeps answering from its last committed state. Clients that need current state
read from a node with `stale: false`, or compare `committed_height` across nodes.

## Event stream

`GET /v1/events/blocks` sends:

- `event: commit_block`, data `{block_hash, timestamp_secs, tx_count, receipt_count, height}`
  (`height` omitted if it could not be read), published after the block's state is written
  (a client can read it immediately);
- `event: lagged`, data `{missed: n}`, when the client fell more than the node's buffer (1024
  events) behind. Events were dropped; re-read `/v1/blocks/latest` and the state you track.

Keep-alive comments are sent on idle streams. The stream has no request timeout.

## Admission

Checked against this node's committed state before an item enters the mempool; cheap checks
first, the signature last. A stale node judges against old state (a nonce or settlement it has
not seen yet) and admission answers carry no freshness label: submit to nodes that report
ready (`/readyz`) or `stale: false` on reads.

| Status | When |
|---|---|
| `400` | invalid transaction: missing sender or recipient, uppercase sender, fee below the floor (1), nonce gap (it must equal the committed nonce or follow the sender's last pending nonce), amount + fee overflowing or above the committed balance, unsigned or forged; invalid receipt: missing task or worker id, coordinator key not listed in genesis or bad coordinator signature, worker not registered with the minimum bond or worker signature not by its registered key, no client or delivery not confirmed, client is the worker's owner, no output tokens, `client_cost` or `worker_reward` not equal to `tokens_computed × price_per_token`, cost above the client's allowance for the coordinator or its committed balance; system transactions: `system:allowance` with an amount or for a key that is not a genesis coordinator, a slash or epoch close by a non-governance key or with an amount, an epoch close before `epoch_seconds` of block time |
| `409` | already pending (duplicate), nonce already committed, task already settled |
| `429` | the sender has 256 pending transactions, or the client account 256 pending receipts; or the client's rate limit (below), with `Retry-After` in whole seconds (at least 1) |
| `503` | mempool full (20 000 transactions, 5 000 receipts; a better-paying transaction evicts the cheapest chain tail) or the node is draining |
| `401` | key file configured and the API key is missing or unknown; `WWW-Authenticate: Bearer` |
| `413` | body over 64 KiB |
| `408` | request not completed within 10 s |

Pending items expire after 15 minutes. A sender's pending transactions are a contiguous nonce
run; block production takes each run in nonce order, ranked by the fee of its next
transaction. A transaction that passes admission but fails execution (e.g. a second transfer
the first one left unaffordable, a malformed system payload) is evicted together with the
sender's later nonces and must be resubmitted. There is no replace-by-fee.

Receipt rules are the chain's settlement rules (`ComputeReceipt` in `src/core/types.rs`, TODO
8.5.1); block execution enforces them again. A settled receipt proves that a genesis
coordinator and a bonded worker signed it and that the client paid `tokens × price` on chain;
it does not prove that the inference was correct or that anyone paid off chain (`proof_hash` is
an opaque anchor). A client authorizes charges in advance: a transaction to `system:allowance` with payload
`{"coordinator": "<hex key>", "amount": n}` sets how much that genesis coordinator may still
charge it, and each settled receipt spends from it. Setting replaces the remainder (0 revokes):
to lower an allowance safely, revoke first, wait for it to commit, then set the new amount;
otherwise receipts settled before and after the change can together exceed either value. A compromised
coordinator key can therefore take at most the allowances clients gave it. A receipt admitted
for a worker that unbonds before settlement is evicted silently (the `202` was only admission).

## Access control

- `--rpc-api-keys <file>`: lines `<name> <sha256-hex-of-token>`, `#` comments; the file must not
  be group- or world-writable; it is read at start (revoke = edit and restart). Create entries
  with `miraset-chain rpc-key --name <name>`, which prints the token (a full credential: hand it
  over securely, keep it out of logs) and the file line. Clients send
  `Authorization: Bearer <token>` on both submission routes. Tokens are meant for server-side
  clients; CORS allows any origin because a bearer header is not an ambient browser credential.
- Without a key file submissions are open and the node logs a warning: devnet only.
- Rate limit per client (`--rpc-submit-rate`, default 50/s; `--rpc-submit-burst`, default
  100): per key name, or without keys per IPv4 address / IPv6 /64. Behind a reverse proxy every
  client shares the proxy's address, so use keys there. Up to 10 000 clients are tracked; others
  share one overflow bucket. The key and the rate limit are checked before the body is read: a
  `401` uses no token, every later refusal (`400`, `409`, `413`, ...) does.
- `miraset_rpc_rejected_total{reason="unauthorized"|"rate_limited"}` and
  `miraset_admission_rejected_total{kind, reason}` count refusals.

`loadgen` sends the token from `MIRASET_RPC_API_KEY`.

## Not in v1

Transaction lookup by hash or status, validator/stake queries, worker task lists, WebSocket
subscriptions and JSON-RPC method names from `docs/HotStuff-rs.md` §5.9 are not implemented;
they are added only when the approved scope needs them. Breaking changes require `/v2`.
