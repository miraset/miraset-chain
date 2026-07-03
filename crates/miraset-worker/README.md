# Miraset Worker
AI inference worker that executes jobs and submits verifiable results to the Miraset blockchain.
## Quick Start
### 1. Start Node
```bash
cargo run --bin miraset -- node start
```
### 2. Start Worker
```bash
cargo run --bin miraset-worker
```
Worker will auto-register on-chain and listen on `http://127.0.0.1:8080`
## API Endpoints
- `GET /health` - Health check
- `GET /status` - Status alias
- `GET /ping` - Lightweight ping
- `POST /jobs/accept` - Accept job assignment
- `POST /jobs/run` - Execute job with prompt
- `GET /jobs/:id/status` - Get job status
- `POST /jobs/:id/report` - Generate signed receipt
- `GET /jobs/:id/stream` - Stream job output
## Configuration
The inference backend is selected via environment variables (no code edit needed):

| Variable | Default | Description |
|---|---|---|
| `MIRASET_BACKEND_TYPE` | `ollama` | Backend selector. See table below. |
| `MIRASET_BACKEND_URL` | per backend | Base URL of the inference engine |
| `MIRASET_BACKEND_API_KEY` | _(unset)_ | Bearer key for cloud OpenAI-compatible providers |
| `MIRASET_SUPPORTED_MODELS` | (per backend defaults) | Comma-separated model id list (overrides defaults) |

### Supported backends

| `MIRASET_BACKEND_TYPE` | Engine | Local/Cloud | Default URL | Default models |
|---|---|---|---|---|
| `ollama` | Ollama (native `/api/generate`) | local | `http://localhost:11434` | `gemma3:4b`, `llama3.3:latest`, `deepseek-r1:8b`, `qwen2.5:7b` |
| `lmstudio` | LM Studio (OpenAI-compatible) | local | `http://localhost:1234/v1` | `gpt-4o-mini`, `gpt-4o`, `gpt-4-turbo`, `gpt-3.5-turbo` |
| `vllm` | vLLM (OpenAI-compatible) | local | `http://localhost:8000/v1` | same as `lmstudio` |
| `localai` | LocalAI (OpenAI-compatible) | local | `http://localhost:8080/v1` | same as `lmstudio` |
| `openai` | OpenAI-compatible (LM Studio, vLLM, LocalAI, OpenAI, Groq, OpenRouter, ...) | local + cloud | `http://localhost:1234/v1` | `gpt-4o-mini`, `gpt-4o`, `gpt-4-turbo`, `gpt-3.5-turbo` |
| `openllm` | BentoML OpenLLM (`/v1/chat/completions`) | local | `http://localhost:3000` | `meta-llama/Llama-3.1-8B-Instruct`, `mistralai/Mistral-7B-Instruct-v0.3`, `Qwen/Qwen2.5-7B-Instruct`, `google/gemma-2-9b-it` |
| `llamacpp` | llama.cpp server (native `/completion`) | local | `http://localhost:8080` | `Qwen/Qwen2.5-7B-Instruct`, `meta-llama/Llama-3.1-8B-Instruct`, `mistralai/Mistral-7B-Instruct-v0.3`, `google/gemma-2-9b-it` |
| `tgi` | HuggingFace text-generation-inference (`/generate`) | local | `http://localhost:8080` | `meta-llama/Llama-3.1-8B-Instruct`, `mistralai/Mistral-7B-Instruct-v0.3`, `google/gemma-2-9b-it`, `Qwen/Qwen2.5-7B-Instruct` |

The `openai` family (`openai`, `lmstudio`, `vllm`, `localai`, `groq`,
`openrouter`, `together`, `fireworks`, `anyscale`) all share the same backend
implementation. Cloud providers require `MIRASET_BACKEND_API_KEY`.

Examples:

```bash
# Ollama (default)
cargo run --bin miraset-worker

# LM Studio
MIRASET_BACKEND_TYPE=lmstudio cargo run --bin miraset-worker

# vLLM
MIRASET_BACKEND_TYPE=vllm cargo run --bin miraset-worker

# LocalAI
MIRASET_BACKEND_TYPE=localai cargo run --bin miraset-worker

# OpenAI cloud
MIRASET_BACKEND_TYPE=openai MIRASET_BACKEND_URL=https://api.openai.com/v1 MIRASET_BACKEND_API_KEY=sk-... cargo run --bin miraset-worker

# Groq cloud
MIRASET_BACKEND_TYPE=groq MIRASET_BACKEND_URL=https://api.groq.com/openai/v1 MIRASET_BACKEND_API_KEY=gsk_... cargo run --bin miraset-worker

# BentoML OpenLLM
MIRASET_BACKEND_TYPE=openllm cargo run --bin miraset-worker

# llama.cpp server
MIRASET_BACKEND_TYPE=llamacpp cargo run --bin miraset-worker

# HuggingFace TGI
MIRASET_BACKEND_TYPE=tgi cargo run --bin miraset-worker

# Override supported models
MIRASET_SUPPORTED_MODELS="llama3.1:8b,mistral:7b" MIRASET_BACKEND_TYPE=ollama cargo run --bin miraset-worker
```

When the configured backend is unreachable, the worker falls back to a
deterministic mock response (devnet/testing friendly).
## End-to-End Flow
1. **Worker Registration**: Worker registers on-chain with GPU specs
2. **Job Assignment**: Accept job via POST `/jobs/accept`
3. **Job Execution**: Run inference via POST `/jobs/run`
4. **Result Submission**: Auto-submit signed receipt to chain
5. **Receipt Anchoring**: Receipt hash anchored on-chain for verification
## Example Usage
### Accept a job
```bash
curl -X POST http://localhost:8080/jobs/accept \
  -H "Content-Type: application/json" \
  -d '{
    "job_id": "0000000000000000000000000000000000000000000000000000000000000001",
    "epoch_id": 1,
    "model_id": "llama2",
    "max_tokens": 100,
    "price_per_token": 10
  }'
```
### Run the job
```bash
curl -X POST http://localhost:8080/jobs/run \
  -H "Content-Type: application/json" \
  -d '{
    "job_id": "0000000000000000000000000000000000000000000000000000000000000001",
    "prompt": "Explain quantum computing",
    "temperature": 0.7
  }'
```
### Check status
```bash
curl http://localhost:8080/jobs/0000000000000000000000000000000000000000000000000000000000000001/status
```
### Generate receipt
```bash
curl -X POST http://localhost:8080/jobs/0000000000000000000000000000000000000000000000000000000000000001/report
```
## Architecture
```
┌──────────────┐      ┌──────────────┐      ┌─────────────────────┐
│ Miraset Node │◄────►│ Miraset      │◄────►│ Inference Backend   │
│   (Chain)    │ RPC  │   Worker     │ HTTP │ ollama/openai/...   │
└──────────────┘      └──────────────┘      └─────────────────────┘
```
## Testing
```bash
cargo test --package miraset-worker
```
