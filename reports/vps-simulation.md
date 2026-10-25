# Namecheap VPS Simulation Report — Miraset Chain

- Generated at: 2026-10-25T14:59:54.213431+00:00
- Source of specs/prices: https://www.namecheap.com/hosting/vps/
- Methodology: node + benchmark inside cgroup scope per package (CPUQuota = package cores, MemoryMax = package RAM); default block interval 300 s.
- Host: Linux x86_64, 16 cores, 30.7 GiB

## Summary

| Package | CPU | RAM | Disk | $/mo | Verdict | Node peak RSS | Scope peak RAM | 10k block | Max tx/interval |
|---|---:|---:|---:|---:|---|---:|---:|---:|---:|
| Hypernova | 12 | 24576 MB | 500 GB | 46.88 | **SUPPORTED** | 8.9 MiB | 24.1 MiB | 436 ms | 6,887,882 |
| Magnetar | 8 | 12288 MB | 240 GB | 24.88 | **SUPPORTED** | 8.8 MiB | 24.3 MiB | 395 ms | 7,590,737 |
| Pulsar | 2 | 2048 MB | 40 GB | 6.88 | **SUPPORTED** | 8.6 MiB | 23.9 MiB | 398 ms | 7,530,264 |
| Quasar | 4 | 6144 MB | 120 GB | 12.88 | **SUPPORTED** | 8.7 MiB | 24.2 MiB | 403 ms | 7,446,059 |
| Spark | 1 | 1024 MB | 20 GB | 3.88 | **SUPPORTED** | 8.4 MiB | 25.0 MiB | 446 ms | 6,728,312 |

## Hypernova — 12 CPU, 24576 MB RAM, 500 GB, $46.88/mo

### Node runtime (under package limits)

- Node started: yes
- Benchmark exit code: 0
- Blocks produced during run (5 s interval): 1 (height 1 → 2)
- Node peak RSS: 8.9 MiB
- Node CPU during bench: 0.0% (0 ms)
- Whole-scope peak RAM (node+bench): 24.1 MiB
- Whole-scope CPU during bench: 5.31 s
- Bench wall time: 5.12 s
- Node storage footprint after run: 1.0 KiB

### Block production benchmark

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.02 | 0 | 0.0 | 0.0 | 118 | 2.9 |
| 1 | 0.04 | 0.04 | 0.05 | 26698 | 0.0 | 0.0 | 350 | 2.9 |
| 100 | 4.16 | 3.84 | 4.53 | 24047 | 40.0 | 94.7 | 23318 | 3.1 |
| 1000 | 41.15 | 37.90 | 46.20 | 24302 | 420.0 | 100.6 | 232118 | 4.4 |
| 10000 | 435.55 | 360.32 | 490.09 | 22960 | 4440.0 | 102.5 | 2320118 | 19.5 |

### Projections & checks

- Sustainable throughput per 300 s block interval: **6,887,882 tx** (mempool cap is 10,000 tx — headroom 688×)
- Chain growth, worst case (every block full, 10k tx): 0.668 GB/day
- Chain growth, light load (1k tx/block): 0.0668 GB/day

| Check | Result | Detail |
|---|---|---|
| RAM fit (peak usage of full scope) | PASS | 24688 KiB / 24576 MB = 0.1% |
| CPU cores for single-threaded block production | PASS | 12 core(s); bench peaks ~100% of one core |
| 10k-tx block latency vs 300s interval | PASS | 436 ms << 300 s interval |
| Storage: days to fill disk (worst case, every block 10k tx) | PASS | 749 days on 500 GB |

**Verdict: SUPPORTED**

## Magnetar — 8 CPU, 12288 MB RAM, 240 GB, $24.88/mo

### Node runtime (under package limits)

- Node started: yes
- Benchmark exit code: 0
- Blocks produced during run (5 s interval): 0 (height 1 → 1)
- Node peak RSS: 8.8 MiB
- Node CPU during bench: 0.0% (0 ms)
- Whole-scope peak RAM (node+bench): 24.3 MiB
- Whole-scope CPU during bench: 4.95 s
- Bench wall time: 4.81 s
- Node storage footprint after run: 880 B

### Block production benchmark

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.01 | 0 | 0.0 | 0.0 | 118 | 2.9 |
| 1 | 0.04 | 0.04 | 0.09 | 27806 | 0.0 | 0.0 | 350 | 2.9 |
| 100 | 4.08 | 3.86 | 4.31 | 24487 | 40.0 | 97.6 | 23318 | 3.1 |
| 1000 | 40.62 | 38.33 | 44.06 | 24621 | 410.0 | 100.9 | 232118 | 4.4 |
| 10000 | 395.22 | 359.68 | 507.34 | 25302 | 4140.0 | 102.0 | 2320118 | 19.7 |

### Projections & checks

- Sustainable throughput per 300 s block interval: **7,590,737 tx** (mempool cap is 10,000 tx — headroom 759×)
- Chain growth, worst case (every block full, 10k tx): 0.668 GB/day
- Chain growth, light load (1k tx/block): 0.0668 GB/day

| Check | Result | Detail |
|---|---|---|
| RAM fit (peak usage of full scope) | PASS | 24876 KiB / 12288 MB = 0.2% |
| CPU cores for single-threaded block production | PASS | 8 core(s); bench peaks ~100% of one core |
| 10k-tx block latency vs 300s interval | PASS | 395 ms << 300 s interval |
| Storage: days to fill disk (worst case, every block 10k tx) | PASS | 359 days on 240 GB |

**Verdict: SUPPORTED**

## Pulsar — 2 CPU, 2048 MB RAM, 40 GB, $6.88/mo

### Node runtime (under package limits)

- Node started: yes
- Benchmark exit code: 0
- Blocks produced during run (5 s interval): 0 (height 1 → 1)
- Node peak RSS: 8.6 MiB
- Node CPU during bench: 0.0% (0 ms)
- Whole-scope peak RAM (node+bench): 23.9 MiB
- Whole-scope CPU during bench: 5.05 s
- Bench wall time: 4.91 s
- Node storage footprint after run: 1.0 KiB

### Block production benchmark

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.02 | 0 | 0.0 | 0.0 | 118 | 2.9 |
| 1 | 0.04 | 0.04 | 0.11 | 24755 | 0.0 | 0.0 | 350 | 3.0 |
| 100 | 4.56 | 4.07 | 5.10 | 21912 | 40.0 | 87.0 | 23318 | 3.1 |
| 1000 | 46.20 | 41.57 | 55.03 | 21645 | 460.0 | 99.6 | 232118 | 4.4 |
| 10000 | 398.39 | 364.30 | 473.32 | 25101 | 4130.0 | 101.9 | 2320118 | 19.5 |

### Projections & checks

- Sustainable throughput per 300 s block interval: **7,530,264 tx** (mempool cap is 10,000 tx — headroom 753×)
- Chain growth, worst case (every block full, 10k tx): 0.668 GB/day
- Chain growth, light load (1k tx/block): 0.0668 GB/day

| Check | Result | Detail |
|---|---|---|
| RAM fit (peak usage of full scope) | PASS | 24488 KiB / 2048 MB = 1.2% |
| CPU cores for single-threaded block production | PASS | 2 core(s); bench peaks ~100% of one core |
| 10k-tx block latency vs 300s interval | PASS | 398 ms << 300 s interval |
| Storage: days to fill disk (worst case, every block 10k tx) | PASS | 60 days on 40 GB |

**Verdict: SUPPORTED**

## Quasar — 4 CPU, 6144 MB RAM, 120 GB, $12.88/mo

### Node runtime (under package limits)

- Node started: yes
- Benchmark exit code: 0
- Blocks produced during run (5 s interval): 0 (height 1 → 1)
- Node peak RSS: 8.7 MiB
- Node CPU during bench: 0.0% (0 ms)
- Whole-scope peak RAM (node+bench): 24.2 MiB
- Whole-scope CPU during bench: 4.97 s
- Bench wall time: 4.85 s
- Node storage footprint after run: 885 B

### Block production benchmark

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.01 | 0 | 0.0 | 0.0 | 118 | 3.0 |
| 1 | 0.04 | 0.04 | 0.06 | 28003 | 0.0 | 0.0 | 350 | 3.0 |
| 100 | 3.68 | 3.53 | 3.83 | 27138 | 40.0 | 109.3 | 23318 | 3.2 |
| 1000 | 36.56 | 35.01 | 44.42 | 27350 | 380.0 | 102.2 | 232118 | 4.5 |
| 10000 | 402.90 | 375.05 | 472.27 | 24820 | 4210.0 | 101.7 | 2320118 | 19.6 |

### Projections & checks

- Sustainable throughput per 300 s block interval: **7,446,059 tx** (mempool cap is 10,000 tx — headroom 744×)
- Chain growth, worst case (every block full, 10k tx): 0.668 GB/day
- Chain growth, light load (1k tx/block): 0.0668 GB/day

| Check | Result | Detail |
|---|---|---|
| RAM fit (peak usage of full scope) | PASS | 24804 KiB / 6144 MB = 0.4% |
| CPU cores for single-threaded block production | PASS | 4 core(s); bench peaks ~100% of one core |
| 10k-tx block latency vs 300s interval | PASS | 403 ms << 300 s interval |
| Storage: days to fill disk (worst case, every block 10k tx) | PASS | 180 days on 120 GB |

**Verdict: SUPPORTED**

## Spark — 1 CPU, 1024 MB RAM, 20 GB, $3.88/mo

### Node runtime (under package limits)

- Node started: yes
- Benchmark exit code: 0
- Blocks produced during run (5 s interval): 1 (height 1 → 2)
- Node peak RSS: 8.4 MiB
- Node CPU during bench: 0.0% (0 ms)
- Whole-scope peak RAM (node+bench): 25.0 MiB
- Whole-scope CPU during bench: 5.38 s
- Bench wall time: 5.39 s
- Node storage footprint after run: 1.0 KiB

### Block production benchmark

| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | block bytes | peak RSS MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 0 | 0.00 | 0.00 | 0.01 | 0 | 0.0 | 0.0 | 118 | 2.9 |
| 1 | 0.04 | 0.04 | 0.05 | 27532 | 0.0 | 0.0 | 350 | 2.9 |
| 100 | 4.13 | 3.74 | 4.47 | 24237 | 40.0 | 98.0 | 23318 | 3.1 |
| 1000 | 42.22 | 39.03 | 54.16 | 23686 | 420.0 | 96.4 | 232118 | 4.4 |
| 10000 | 445.88 | 432.53 | 487.56 | 22428 | 4470.0 | 98.9 | 2320118 | 19.5 |

### Projections & checks

- Sustainable throughput per 300 s block interval: **6,728,312 tx** (mempool cap is 10,000 tx — headroom 672×)
- Chain growth, worst case (every block full, 10k tx): 0.668 GB/day
- Chain growth, light load (1k tx/block): 0.0668 GB/day

| Check | Result | Detail |
|---|---|---|
| RAM fit (peak usage of full scope) | PASS | 25560 KiB / 1024 MB = 2.4% |
| CPU cores for single-threaded block production | PASS | 1 core(s); bench peaks ~100% of one core |
| 10k-tx block latency vs 300s interval | PASS | 446 ms << 300 s interval |
| Storage: days to fill disk (worst case, every block 10k tx) | WARN | 30 days on 20 GB |

**Verdict: SUPPORTED**

## Assumptions & limitations

- Methodology: node + benchmark run inside a systemd cgroup scope with CPUQuota/MemoryMax equal to the package specs (cgroup v2). This emulates CPU/RAM limits of the VPS, NOT its actual CPU generation or NVMe speed; absolute numbers will differ on real Namecheap hardware.
- Node: `miraset node start`, block interval 5 s during the simulation, fresh Sled storage in a temp dir (storage footprint measured after run).
- Benchmark: 10 repeats per scenario, in-memory state, cheap gas config.
- Price/specs source: namecheap.com/hosting/vps (2026).
