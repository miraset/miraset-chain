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

## Examples

```bash
curl http://127.0.0.1:9944/health
curl http://127.0.0.1:9944/block/latest
curl http://127.0.0.1:9944/balance/<hex-address>
```

