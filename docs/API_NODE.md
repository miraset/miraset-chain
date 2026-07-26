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
Submit a signed `Transaction::CreateJob` to the transaction pipeline. The job is included in the next produced block and auto-assigned to a suitable active worker during block execution. An HTTP `/jobs/accept` notification is dispatched to the worker after the block is produced.

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

The actual `job_id` and `assigned_worker` are only known once the block is produced. Poll `GET /jobs` or `GET /jobs/{id}` (filtering by requester) to check status.

### `GET /jobs/{id}`
Get a single job by hex job ID.

## Workers

### `GET /workers`
List all registered workers.

Response: array of `WorkerView`
```json
[
  {
    "worker_id": "...",
    "owner": "...",
    "gpu_model": "NVIDIA RTX 4090",
    "vram_gib": 24,
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

## Examples

```bash
curl http://127.0.0.1:9944/health
curl http://127.0.0.1:9944/block/latest
curl http://127.0.0.1:9944/balance/<hex-address>
curl http://127.0.0.1:9944/jobs
curl http://127.0.0.1:9944/workers
curl http://127.0.0.1:9944/epoch
```

