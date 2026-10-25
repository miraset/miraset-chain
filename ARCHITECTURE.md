# Architecture

Miraset is a Rust workspace with a single-author devnet node, a separate
inference worker, and a desktop wallet. The node owns transaction validation,
block production, job assignment, and the node RPC API. Workers register with
the node, advertise supported models, and execute assigned jobs through a
pluggable inference backend.

## Local inference test

The local Docker test keeps the node as the job coordinator and runs one
worker container plus Ollama from the official `ollama/ollama` image. The
worker calls the Ollama service over the private Compose network. Ollama model
data persists in a Docker volume; a one-shot service pulls the configured
model before the worker starts. CPU inference is the default; GPU passthrough
is optional and requires the matching host container runtime/toolkit. Clients
submit a signed `CreateJob` transaction together with a separately signed,
off-chain prompt through the node RPC. The prompt is not added to the
transaction or block serialization. The node holds it in memory until dispatch
to the assigned worker, and the worker executes Ollama and submits the result
and receipt hash back to the node.

This in-memory input handoff is for local testing only: restarting the node
before dispatch loses the prompt. The dev Docker configuration explicitly
allows private worker endpoints and must not be exposed as a public deployment.

See [ADR 0001](docs/adr/0001-local-off-chain-job-input.md) for the decision.
