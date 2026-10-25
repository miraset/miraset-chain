# Block Resource Benchmark

- Generated at: 2026-10-25T14:58:47.426951231+00:00
- Target package: Quasar (4 CPU cores, 6144 MB RAM, 120 GB storage, $12.88/mo)
- OS: linux
- CPU cores: 4
- Repeats per scenario: 10
- System RAM: 32165740 KiB
- Total CPU time across scenarios: 4630.0 ms
- Max txs per block (mempool cap): 10000

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS KiB | RSS delta KiB | RAM % |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.01 | 0.0 | 0.0 | 0.0 | 118 | 3060 | 144 | 0.010 |
| 1 | 0.04 | 0.04 | 0.06 | 28003.4 | 0.0 | 0.0 | 350 | 3076 | 16 | 0.010 |
| 100 | 3.68 | 3.53 | 3.83 | 27137.9 | 40.0 | 109.3 | 23318 | 3240 | 124 | 0.010 |
| 1000 | 36.56 | 35.01 | 44.42 | 27350.1 | 380.0 | 102.2 | 232118 | 4560 | 4 | 0.014 |
| 10000 | 402.90 | 375.05 | 472.27 | 24820.2 | 4210.0 | 101.7 | 2320118 | 20072 | 0 | 0.062 |

Notes: `0` = empty-block baseline; `10000` = mempool capacity (practical max txs per block; block production itself has no tx cap). In-memory state, no storage/RPC. Cheap gas config. `CPU ms` is the total process CPU time (user + system) for the whole scenario including all repeats and setup; `CPU %` = CPU ms / summed wall time of the timed repeats (may exceed 100% slightly because setup work is included in CPU but not in wall samples). `RAM %` is peak process RSS relative to total system RAM.
