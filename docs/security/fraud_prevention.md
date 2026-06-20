# Miraset Chain Security

## Fraud Prevention Mechanisms
1. **Receipt Anchoring**: Cryptographic commitments on-chain.
2. **Dual Signatures**: Worker + Coordinator signatures.
3. **Challenge Window**: Period for dispute submission.
4. **Slashing Mechanism**: Proven fraud results in stake loss.

## Threat Mitigation
| Threat | Control |
|--------|---------|
| Fraudulent reporting | Signed receipts + coordinator co-signature |
| Uptime spoofing | Independent validator sampling |
| Coordinator abuse | Transparent assignment rules |
| Replay attacks | Job ID uniqueness |
