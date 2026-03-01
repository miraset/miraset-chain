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
Edit `src/main.rs`:
```rust
WorkerConfig {
    endpoint: "127.0.0.1:8080".to_string(),
    node_url: "http://127.0.0.1:9944".to_string(),
    ollama_url: "http://localhost:11434".to_string(),
    gpu_model: "NVIDIA RTX 4090".to_string(),
    vram_total_gib: 24,
    supported_models: vec!["llama2".to_string()],
}
```
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
┌──────────────┐      ┌──────────────┐      ┌─────────────┐
│ Miraset Node │◄────►│ Miraset      │◄────►│   Ollama    │
│   (Chain)    │ RPC  │   Worker     │ HTTP │  Backend    │
└──────────────┘      └──────────────┘      └─────────────┘
```
## Testing
```bash
cargo test --package miraset-worker
```
