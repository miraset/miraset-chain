# Block Resource Benchmark

- Generated at: 2026-10-25T14:58:42.420836385+00:00
- Target package: Pulsar (2 CPU cores, 2048 MB RAM, 40 GB storage, $6.88/mo)
- OS: linux
- CPU cores: 2
- Repeats per scenario: 10
- System RAM: 32165740 KiB
- Total CPU time across scenarios: 4630.0 ms
- Max txs per block (mempool cap): 10000

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS KiB | RSS delta KiB | RAM % |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.02 | 0.0 | 0.0 | 0.0 | 118 | 2972 | 80 | 0.009 |
| 1 | 0.04 | 0.04 | 0.11 | 24754.9 | 0.0 | 0.0 | 350 | 3052 | 16 | 0.009 |
| 100 | 4.56 | 4.07 | 5.10 | 21911.6 | 40.0 | 87.0 | 23318 | 3216 | 124 | 0.010 |
| 1000 | 46.20 | 41.57 | 55.03 | 21644.8 | 460.0 | 99.6 | 232118 | 4540 | 4 | 0.014 |
| 10000 | 398.39 | 364.30 | 473.32 | 25100.9 | 4130.0 | 101.9 | 2320118 | 19996 | 0 | 0.062 |

Notes: `0` = empty-block baseline; `10000` = mempool capacity (practical max txs per block; block production itself has no tx cap). In-memory state, no storage/RPC. Cheap gas config. `CPU ms` is the total process CPU time (user + system) for the whole scenario including all repeats and setup; `CPU %` = CPU ms / summed wall time of the timed repeats (may exceed 100% slightly because setup work is included in CPU but not in wall samples). `RAM %` is peak process RSS relative to total system RAM.
