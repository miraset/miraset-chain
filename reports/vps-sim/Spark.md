# Block Resource Benchmark

- Generated at: 2026-10-25T14:58:32.026094706+00:00
- Target package: Spark (1 CPU cores, 1024 MB RAM, 20 GB storage, $3.88/mo)
- OS: linux
- CPU cores: 1
- Repeats per scenario: 10
- System RAM: 32165740 KiB
- Total CPU time across scenarios: 4930.0 ms
- Max txs per block (mempool cap): 10000

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS KiB | RSS delta KiB | RAM % |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.01 | 0.0 | 0.0 | 0.0 | 118 | 2996 | 208 | 0.009 |
| 1 | 0.04 | 0.04 | 0.05 | 27531.5 | 0.0 | 0.0 | 350 | 3012 | 16 | 0.009 |
| 100 | 4.13 | 3.74 | 4.47 | 24236.8 | 40.0 | 98.0 | 23318 | 3176 | 124 | 0.010 |
| 1000 | 42.22 | 39.03 | 54.16 | 23686.4 | 420.0 | 96.4 | 232118 | 4496 | 4 | 0.014 |
| 10000 | 445.88 | 432.53 | 487.56 | 22427.7 | 4470.0 | 98.9 | 2320118 | 19992 | 0 | 0.062 |

Notes: `0` = empty-block baseline; `10000` = mempool capacity (practical max txs per block; block production itself has no tx cap). In-memory state, no storage/RPC. Cheap gas config. `CPU ms` is the total process CPU time (user + system) for the whole scenario including all repeats and setup; `CPU %` = CPU ms / summed wall time of the timed repeats (may exceed 100% slightly because setup work is included in CPU but not in wall samples). `RAM %` is peak process RSS relative to total system RAM.
