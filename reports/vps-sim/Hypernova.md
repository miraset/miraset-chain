# Block Resource Benchmark

- Generated at: 2026-10-25T14:58:57.806297468+00:00
- Target package: Hypernova (12 CPU cores, 24576 MB RAM, 500 GB storage, $46.88/mo)
- OS: linux
- CPU cores: 12
- Repeats per scenario: 10
- System RAM: 32165740 KiB
- Total CPU time across scenarios: 4900.0 ms
- Max txs per block (mempool cap): 10000

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS KiB | RSS delta KiB | RAM % |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.02 | 0.0 | 0.0 | 0.0 | 118 | 3000 | 144 | 0.009 |
| 1 | 0.04 | 0.04 | 0.05 | 26698.0 | 0.0 | 0.0 | 350 | 3016 | 16 | 0.009 |
| 100 | 4.16 | 3.84 | 4.53 | 24047.3 | 40.0 | 94.7 | 23318 | 3180 | 124 | 0.010 |
| 1000 | 41.15 | 37.90 | 46.20 | 24302.3 | 420.0 | 100.6 | 232118 | 4504 | 4 | 0.014 |
| 10000 | 435.55 | 360.32 | 490.09 | 22959.6 | 4440.0 | 102.5 | 2320118 | 20016 | 0 | 0.062 |

Notes: `0` = empty-block baseline; `10000` = mempool capacity (practical max txs per block; block production itself has no tx cap). In-memory state, no storage/RPC. Cheap gas config. `CPU ms` is the total process CPU time (user + system) for the whole scenario including all repeats and setup; `CPU %` = CPU ms / summed wall time of the timed repeats (may exceed 100% slightly because setup work is included in CPU but not in wall samples). `RAM %` is peak process RSS relative to total system RAM.
