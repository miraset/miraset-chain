# MIRASET Node API

Base URL: `http://127.0.0.1:9944`

## Status

### `GET /health`
Returns node health with latest block height.

Response:
```json
{
  "status": "healthy",
  "timestamp": "2026-03-01T12:34:56Z",
  "latest_block_height": 42
}
```

### `GET /status`
Alias of `/health`.

### `GET /ping`
Lightweight ping.

Response:
```json
{
  "status": "ok"
}
```

## Chain Data

### `GET /block/latest`
Returns the latest block.

### `GET /block/{height}`
Returns the block at the given height.

### `GET /balance/{address}`
Returns the balance for a hex address.

### `GET /nonce/{address}`
Returns the nonce for a hex address.

### `GET /events?from_height={u64}&limit={usize}`
Returns recent events.

### `GET /chat/messages?limit={usize}`
Returns recent chat messages.

## Transactions

### `POST /tx/submit`
Submits a transaction.

Request body: JSON-serialized `Transaction` from `miraset_core`.

Response:
- `200 OK` on success
- `400 Bad Request` with error message on failure

## Job Coordinator

### `GET /jobs`
List all on-chain inference jobs.

Response: array of `JobView`
```json
[
  {
    "job_id": "...",
    "epoch_id": 0,
    "requester": "...",
    "model_id": "llama-3-8b",
    "max_tokens": 1000,
    "escrow_amount": 10000,
    "status": "Created",
    "assigned_worker": null,
    "created_at": "2026-03-01T12:34:56Z"
  }
]
```

### `POST /jobs/submit`
Submit a signed `Transaction::CreateJob` without an inference prompt. The job
is included in the next produced block and auto-assigned to a suitable active
worker. This legacy route does not start inference by itself.

Request body: a signed `Transaction::CreateJob` JSON object (`miraset_core::Transaction`).
```json
{
  "type": "CreateJob",
  "requester": "<hex-address>",
  "model_id": "llama-3-8b",
  "max_tokens": 1000,
  "escrow_amount": 10000,
  "nonce": 0,
  "signature": "<hex-64-byte-signature>"
}
```

Response:
```json
{
  "status": "accepted",
  "tx_hash": "...",
  "requester": "<hex-address>",
  "model_id": "llama-3-8b"
}
```

The response includes the transaction hash. Poll `GET /jobs` to find the job
after block production.

### `GET /jobs/{id}`
Get a single job by hex job ID.

### `POST /jobs/submit-with-input`
Submit a signed `CreateJob` transaction and a separately signed prompt. The
prompt is delivered to the assigned worker out-of-band and is not included in
the transaction or block. The prompt signature uses
`miraset_core::job_input_signing_bytes(tx_hash, prompt)` and the requester's
Ed25519 key. Prompts must be non-empty and no larger than 64 KiB.

Request:
```json
{
  "transaction": {
    "type": "CreateJob",
    "requester": "<hex-address>",
    "model_id": "gemma3:4b",
    "max_tokens": 128,
    "escrow_amount": 10000,
    "nonce": 0,
    "signature": "<hex-64-byte-signature>"
  },
  "prompt": "Explain what a blockchain node does.",
  "input_signature": "<hex-64-byte-signature>"
}
```

The node verifies both signatures, queues the transaction and prompt together,
and returns the deterministic job ID:
```json
{
  "status": "accepted",
  "tx_hash": "...",
  "job_id": "...",
  "requester": "<hex-address>",
  "model_id": "gemma3:4b"
}
```

The prompt is held in node memory only until job completion; a node restart
before dispatch loses it. The local test uses a shared node-worker dispatch
secret, which also authenticates the prompt digest during assignment.

### `GET /jobs/{id}/result`
Returns the completed worker response through the node RPC. Returns `409` if
the job is not yet complete and `502` if the worker result cannot be reached.

## Workers

### `GET /workers`
List all registered workers.

Response: array of `WorkerView`
```json
[
  {
    "worker_id": "...",
    "owner": "...",
    "gpu_model": "Ollama host (hardware unspecified)",
    "vram_gib": 0,
    "status": "Active",
    "endpoints": ["http://localhost:8080"],
    "supported_models": ["llama-3-8b"]
  }
]
```

## Epoch

### `GET /epoch`
Returns the current epoch status, including verified tokens and counts.

Response:
```json
{
  "id": 0,
  "status": "Submit",
  "start_time": "2026-03-01T12:00:00Z",
  "end_time": "2026-03-01T13:00:00Z",
  "total_verified_tokens": 0,
  "workers_count": 0,
  "jobs_count": 0
}
```

## Limitations

- This node is a single-author devnet: no consensus, no validator set, and block production is local only.
- The `/jobs/submit` endpoint now requires a signed `Transaction::CreateJob` and routes through the same `/tx/submit` pipeline as all other state-mutating operations.
- The test-only Docker configuration enables private worker endpoints. Do not expose it as a public deployment; the node RPC does not provide production-grade client authentication.

## Examples

```bash
curl http://127.0.0.1:9944/health
curl http://127.0.0.1:9944/block/latest
curl http://127.0.0.1:9944/balance/<hex-address>
curl http://127.0.0.1:9944/jobs
curl http://127.0.0.1:9944/workers
curl http://127.0.0.1:9944/epoch
```
