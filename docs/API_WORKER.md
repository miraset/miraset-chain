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

## Backend configuration

The worker talks to a pluggable inference backend (local or cloud) selected via
environment variables. The chain protocol, receipts, and job API are identical
regardless of backend — only the inference engine differs.

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `MIRASET_BACKEND_TYPE` | `ollama` | Backend selector. Accepted (case-insensitive): `ollama`; OpenAI-compatible family: `openai`, `lmstudio`, `vllm`, `localai`, `groq`, `openrouter`, `together`, `fireworks`, `anyscale`; `openllm` (alias `bentoml`); `llamacpp`; `tgi`. |
| `MIRASET_BACKEND_URL` | (per backend) | Base URL of the inference backend. See defaults below. |
| `MIRASET_BACKEND_API_KEY` | _(unset)_ | Optional Bearer auth key. Required for cloud OpenAI-compatible providers; ignored by local backends without auth. |
| `MIRASET_SUPPORTED_MODELS` | (per backend defaults) | Comma-separated list of model ids this worker accepts. Overrides per-backend defaults. |

### Default `MIRASET_BACKEND_URL` per backend

| `MIRASET_BACKEND_TYPE` | Default URL |
|---|---|
| `ollama` | `http://localhost:11434` |
| `openai` | `http://localhost:1234/v1` (LM Studio) |
| `lmstudio` | `http://localhost:1234/v1` |
| `vllm` | `http://localhost:8000/v1` |
| `localai` | `http://localhost:8080/v1` |
| `openllm` (BentoML) | `http://localhost:3000` |
| `llamacpp` | `http://localhost:8080` |
| `tgi` | `http://localhost:8080` |

### Default model sets per backend

Each backend ships with a curated default model set in
`BackendType::default_models()`. Override at runtime via
`MIRASET_SUPPORTED_MODELS`.

| Backend | Default models |
|---|---|
| `ollama` | `gemma3:4b`, `llama3.3:latest`, `deepseek-r1:8b`, `qwen2.5:7b` |
| `openai` (cloud) | `gpt-4o-mini`, `gpt-4o`, `gpt-4-turbo`, `gpt-3.5-turbo` |
| `lmstudio` / `vllm` / `localai` | `gpt-4o-mini`, `gpt-4o`, `gpt-4-turbo`, `gpt-3.5-turbo` |
| `openllm` | `meta-llama/Llama-3.1-8B-Instruct`, `mistralai/Mistral-7B-Instruct-v0.3`, `Qwen/Qwen2.5-7B-Instruct`, `google/gemma-2-9b-it` |
| `llamacpp` | `Qwen/Qwen2.5-7B-Instruct`, `meta-llama/Llama-3.1-8B-Instruct`, `mistralai/Mistral-7B-Instruct-v0.3`, `google/gemma-2-9b-it` |
| `tgi` | `meta-llama/Llama-3.1-8B-Instruct`, `mistralai/Mistral-7B-Instruct-v0.3`, `google/gemma-2-9b-it`, `Qwen/Qwen2.5-7B-Instruct` |

### Supported backends

#### `ollama` — Ollama (native)
Local Ollama daemon. Endpoints used: `POST /api/generate`, `GET /api/tags`.
No API key required.

```bash
MIRASET_BACKEND_TYPE=ollama \
MIRASET_BACKEND_URL=http://localhost:11434 \
cargo run --bin miraset-worker
```

#### `lmstudio` — LM Studio (OpenAI-compatible)
LM Studio's local server. Endpoints used: `POST {base}/v1/chat/completions`,
`GET {base}/v1/models`. No API key.

```bash
MIRASET_BACKEND_TYPE=lmstudio \
cargo run --bin miraset-worker
# uses http://localhost:1234/v1 by default
```

#### `vllm` — vLLM (OpenAI-compatible)
vLLM's OpenAI-compatible server. Endpoints used: same as LM Studio.
No API key.

```bash
MIRASET_BACKEND_TYPE=vllm \
cargo run --bin miraset-worker
# uses http://localhost:8000/v1 by default
```

#### `localai` — LocalAI (OpenAI-compatible)
LocalAI. Endpoints used: same as LM Studio. No API key.

```bash
MIRASET_BACKEND_TYPE=localai \
cargo run --bin miraset-worker
# uses http://localhost:8080/v1 by default
```

#### `openai` — Generic OpenAI-compatible (local and cloud)
Any endpoint implementing the OpenAI Chat Completions + Models API. Covers
local servers (LM Studio, vLLM, LocalAI, llama.cpp OpenAI shim, TGI OpenAI
shim) and cloud providers (OpenAI, Groq, OpenRouter, Together, Anyscale,
Fireworks). Endpoints used: `POST {base}/v1/chat/completions`,
`GET {base}/v1/models`. The `base_url` must include the `/v1` path.

Cloud providers require `MIRASET_BACKEND_API_KEY`.

```bash
# OpenAI (cloud)
MIRASET_BACKEND_TYPE=openai \
MIRASET_BACKEND_URL=https://api.openai.com/v1 \
MIRASET_BACKEND_API_KEY=sk-... \
cargo run --bin miraset-worker

# Groq (cloud)
MIRASET_BACKEND_TYPE=groq \
MIRASET_BACKEND_URL=https://api.groq.com/openai/v1 \
MIRASET_BACKEND_API_KEY=gsk_... \
cargo run --bin miraset-worker

# OpenRouter (cloud, multi-model)
MIRASET_BACKEND_TYPE=openrouter \
MIRASET_BACKEND_URL=https://openrouter.ai/api/v1 \
MIRASET_BACKEND_API_KEY=sk-or-... \
cargo run --bin miraset-worker
```

#### `openllm` — BentoML OpenLLM
Native OpenLLM (BentoML). Endpoints used: `POST {base}/v1/chat/completions`
(OpenAI-compatible surface, which OpenLLM also exposes), `GET {base}/v1/models`.
No API key.

```bash
MIRASET_BACKEND_TYPE=openllm \
cargo run --bin miraset-worker
# uses http://localhost:3000 by default
```

#### `llamacpp` — llama.cpp server (native)
Native llama.cpp HTTP server. Endpoints used: `POST /completion`,
`GET /props`, `GET /health`. No API key. llama.cpp loads one model at server
start and ignores the per-request `model` field; the worker logs a warning if
the requested model name doesn't match the server's loaded model and proceeds
with the loaded model.

```bash
MIRASET_BACKEND_TYPE=llamacpp \
MIRASET_BACKEND_URL=http://localhost:8080 \
cargo run --bin miraset-worker
```

#### `tgi` — HuggingFace text-generation-inference
Native TGI server. Endpoints used: `POST /generate`, `GET /info`. No API key.
TGI serves one model per instance; `/info` exposes the loaded model id.

```bash
MIRASET_BACKEND_TYPE=tgi \
MIRASET_BACKEND_URL=http://localhost:8080 \
cargo run --bin miraset-worker
```

### Mock fallback

When the configured backend is unreachable or returns an error, the worker
falls back to a deterministic mock response (matching the pre-existing
Ollama behavior). This keeps the worker operational for devnet/testing even
when no inference server is running.

