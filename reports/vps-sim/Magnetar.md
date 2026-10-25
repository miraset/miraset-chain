# Block Resource Benchmark

- Generated at: 2026-10-25T14:58:52.428994081+00:00
- Target package: Magnetar (8 CPU cores, 12288 MB RAM, 240 GB storage, $24.88/mo)
- OS: linux
- CPU cores: 8
- Repeats per scenario: 10
- System RAM: 32165740 KiB
- Total CPU time across scenarios: 4590.0 ms
- Max txs per block (mempool cap): 10000

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS KiB | RSS delta KiB | RAM % |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.01 | 0.0 | 0.0 | 0.0 | 118 | 2988 | 144 | 0.009 |
| 1 | 0.04 | 0.04 | 0.09 | 27805.6 | 0.0 | 0.0 | 350 | 3004 | 16 | 0.009 |
| 100 | 4.08 | 3.86 | 4.31 | 24486.9 | 40.0 | 97.6 | 23318 | 3168 | 124 | 0.010 |
| 1000 | 40.62 | 38.33 | 44.06 | 24621.3 | 410.0 | 100.9 | 232118 | 4556 | 4 | 0.014 |
| 10000 | 395.22 | 359.68 | 507.34 | 25302.5 | 4140.0 | 102.0 | 2320118 | 20164 | 0 | 0.063 |

Notes: `0` = empty-block baseline; `10000` = mempool capacity (practical max txs per block; block production itself has no tx cap). In-memory state, no storage/RPC. Cheap gas config. `CPU ms` is the total process CPU time (user + system) for the whole scenario including all repeats and setup; `CPU %` = CPU ms / summed wall time of the timed repeats (may exceed 100% slightly because setup work is included in CPU but not in wall samples). `RAM %` is peak process RSS relative to total system RAM.
