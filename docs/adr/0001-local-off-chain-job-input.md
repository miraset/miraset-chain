# ADR 0001: Keep job prompts off-chain for the local inference flow

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

The node already validates signed `CreateJob` transactions, assigns jobs to
registered workers, and dispatches `/jobs/accept`. The worker already supports
Ollama, but the assignment currently contains no prompt and does not start
inference. Putting prompt text into `Transaction::CreateJob` would change a
signed and bincode-serialized protocol type and permanently expose prompts in
blocks.

## Decision

Keep the node as the coordinator and add a local-test RPC request that pairs
the existing signed `CreateJob` transaction with a separately signed prompt.
The prompt is stored only in node memory until it is dispatched to the worker.
The worker starts inference after accepting the assignment and submits the
result and receipt hash through the existing transaction pipeline. A client
can poll the node RPC for job status and retrieve the result through the node.

The Docker test uses one node and one worker; Ollama runs on the host and is
reached from the worker container through the Docker host gateway. No separate
coordinator service or database is introduced.

## Consequences

- Existing transaction and block serialization remains unchanged.
- Prompt contents are not persisted or consensus-committed; a node restart
  before dispatch loses the input. This is not a production data model.
- The input signature binds the off-chain prompt to the signed job transaction,
  and the node-worker dispatch authentication tag covers the prompt digest.
- The local compose configuration permits private endpoints and disables Ollama
  mock fallback so an unavailable inference backend cannot look successful.
- The devnet is still single-author and its node RPC does not provide
  production-grade client authentication.
