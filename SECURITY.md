# Security policy and proposed threat controls

Status: Draft threat model; no implemented blockchain or completed security
review. No production deployment is supported by this documentation.

## Reporting

Do not post exploitable details, private keys, credentials, prompts or private
inference outputs in public issues. Use the repository's private vulnerability
reporting mechanism if enabled. No private reporting contact has been verified;
obtain an authorized private channel from the maintainer before disclosure.

## Assets and trust boundaries

Protect finalized history, balances/accounting, committee identity, vote/lock
safety state, validator signing keys and RPC credentials.
Network inputs, RPC callers, peers and restored files are untrusted until
validated. A peer TLS identity is not automatically a consensus authorization.
The master account cannot bypass quorum. Workers are not validators.

Assume at most f Byzantine validators for a specified n = 3f + 1 equal-weight
committee, plus arbitrary delay/partition before stabilization. Honest signing
keys and durable safety state must remain uncompromised. State safety under
the bound and liveness after stabilization are distinct claims requiring tests.
Storage durability assumes a documented filesystem/backend failure model.
Compromising more than the fault bound is outside that consensus guarantee.

## Required controls before a release

- Verify signature domains, canonical bytes, strict key validation, distinct
  signer accounting, replay protection and approved cryptographic scopes.
- Persist safety state before required votes; durably commit before publication.
  Fail closed on history corruption and ambiguous storage completion.
- Authenticate and authorize settlement, registry and peer operations
  independently; rate-limit and bound decoding, queues and verification.
- Load keys through protected adapters, validate permissions and configuration,
  keep secrets out of logs, and document rotation/recovery.
- Exclude prompts and private inference output from signed and durable data.
- Test overload, crash, equivocation, partitions, corruption and reconfiguration.
- Run SAST, dependency and secret scans and independent security review; record
  exact tooling, revision and findings rather than assuming clean results.
- Obtain human Security and Code Review approvals and deployment authorization.

No controls above are claimed implemented. Library selection, BLS usage and
safe committee activation remain subject to source verification and review.
