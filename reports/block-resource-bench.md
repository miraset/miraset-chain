# Block Resource Benchmark

- Generated at: 2026-10-25T14:51:01.407934977+00:00
- OS: linux
- CPU cores: 16
- Repeats per scenario: 10
- System RAM: 32165740 KiB
- Total CPU time across scenarios: 4010.0 ms
- Max txs per block (mempool cap): 10000

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS KiB | RSS delta KiB | RAM % |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.01 | 0.0 | 0.0 | 0.0 | 118 | 3036 | 144 | 0.009 |
| 1 | 0.04 | 0.03 | 0.05 | 28374.4 | 0.0 | 0.0 | 350 | 3052 | 16 | 0.009 |
| 100 | 3.48 | 3.45 | 3.68 | 28736.6 | 30.0 | 85.2 | 23318 | 3216 | 124 | 0.010 |
| 1000 | 34.95 | 34.28 | 35.46 | 28608.3 | 350.0 | 100.3 | 232118 | 4604 | 4 | 0.014 |
| 10000 | 355.94 | 351.90 | 362.03 | 28094.5 | 3630.0 | 101.8 | 2320118 | 20176 | 0 | 0.063 |

Notes: `0` = empty-block baseline; `10000` = mempool capacity (practical max txs per block; block production itself has no tx cap). In-memory state, no storage/RPC. Cheap gas config. `CPU ms` is the total process CPU time (user + system) for the whole scenario including all repeats and setup; `CPU %` = CPU ms / summed wall time of the timed repeats (may exceed 100% slightly because setup work is included in CPU but not in wall samples). `RAM %` is peak process RSS relative to total system RAM.
