# Block Resource Benchmark

- Generated at: 2026-10-25T13:41:04.957112497+00:00
- OS: linux
- CPU cores: 16
- Repeats per scenario: 10
- Max txs per block (mempool cap): 10000

| txs/block | median ms | min ms | max ms | tx/s | block bytes | peak RSS KiB | RSS delta KiB |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.01 | 0.0 | 118 | 2848 | 148 |
| 1 | 0.03 | 0.03 | 0.05 | 29216.7 | 350 | 2864 | 12 |
| 100 | 3.52 | 3.51 | 3.64 | 28373.1 | 23318 | 3024 | 124 |
| 1000 | 34.65 | 34.31 | 35.57 | 28859.8 | 232118 | 4416 | 4 |
| 10000 | 348.22 | 346.30 | 354.19 | 28717.3 | 2320118 | 19984 | 0 |

Notes: `0` = empty-block baseline; `10000` = mempool capacity (practical max txs per block; block production itself has no tx cap). In-memory state, no storage/RPC. Cheap gas config.
