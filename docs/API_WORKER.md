# MIRASET Worker API

Base URL: `http://127.0.0.1:8080`

## Status

### `GET /health`
Returns worker health.

Response:
```json
{
  "status": "healthy",
  "timestamp": "2026-03-01T12:34:56Z"
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

## Jobs

### `POST /jobs/accept`
Accept a job assignment.

Request body:
```json
{
  "job_id": "<64-hex>",
  "epoch_id": 1,
  "model_id": "llama2",
  "max_tokens": 100,
  "price_per_token": 10
}
```

### `POST /jobs/run`
Run an accepted job.

Request body:
```json
{
  "job_id": "<64-hex>",
  "prompt": "Hello",
  "temperature": 0.7,
  "top_p": 0.9
}
```

### `GET /jobs/{id}/status`
Returns full job state.

### `POST /jobs/{id}/report`
Returns the signed job receipt.

### `GET /jobs/{id}/stream`
Returns final response tokens and output metadata.

## Examples

```bash
curl http://127.0.0.1:8080/health
curl -X POST http://127.0.0.1:8080/jobs/accept \
  -H "Content-Type: application/json" \
  -d '{"job_id":"0000000000000000000000000000000000000000000000000000000000000042","epoch_id":1,"model_id":"llama2","max_tokens":100,"price_per_token":10}'
```

