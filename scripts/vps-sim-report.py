#!/usr/bin/env python3
"""Aggregate Namecheap VPS package simulation results into reports/vps-simulation.{md,json}.

Inputs (written by scripts/vps-sim-run.sh):
  reports/vps-sim/<Package>.json        - block resource bench report
  reports/vps-sim/<Package>.node.json   - node runtime metrics under cgroup limits
"""

from __future__ import annotations

import datetime
import glob
import json
import os
import platform

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIM_DIR = os.path.join(ROOT, "reports", "vps-sim")
OUT_MD = os.path.join(ROOT, "reports", "vps-simulation.md")
OUT_JSON = os.path.join(ROOT, "reports", "vps-simulation.json")

# Repo default block interval (seconds) — see default_block_interval() in miraset-cli.
DEFAULT_BLOCK_INTERVAL_S = 300
SECONDS_PER_DAY = 86_400

# Projection assumptions used in the report.
NOTES = """\
- Methodology: node + benchmark run inside a systemd cgroup scope with CPUQuota/MemoryMax equal to the package specs (cgroup v2). This emulates CPU/RAM limits of the VPS, NOT its actual CPU generation or NVMe speed; absolute numbers will differ on real Namecheap hardware.
- Node: `miraset node start`, block interval 5 s during the simulation, fresh Sled storage in a temp dir (storage footprint measured after run).
- Benchmark: 10 repeats per scenario, in-memory state, cheap gas config.
- Price/specs source: namecheap.com/hosting/vps (2026).
"""


def load_packages() -> list[dict]:
    packages = []
    for bench_path in sorted(glob.glob(os.path.join(SIM_DIR, "*.json"))):
        if bench_path.endswith(".node.json"):
            continue
        node_path = bench_path[:-len(".json")] + ".node.json"
        with open(bench_path) as f:
            bench = json.load(f)
        node = {}
        if os.path.exists(node_path):
            with open(node_path) as f:
                node = json.load(f)
        pkg = (bench.get("package") or {}).get("name") or node.get("package")
        if not pkg:
            continue
        packages.append({"name": pkg, "bench": bench, "node": node})
    return packages


def fmt_bytes(v) -> str:
    if v is None:
        return "-"
    if v < 1024:
        return f"{v} B"
    if v < 1024**2:
        return f"{v / 1024:.1f} KiB"
    if v < 1024**3:
        return f"{v / 1024**2:.2f} MiB"
    return f"{v / 1024**3:.2f} GiB"


def scenario_map(bench: dict) -> dict[int, dict]:
    return {s["tx_count"]: s for s in bench.get("scenarios", [])}


def fmt(v, suffix="", nd=2):
    if v is None:
        return "-"
    if isinstance(v, float):
        return f"{v:.{nd}f}{suffix}"
    return f"{v}{suffix}"


def evaluate(pkg: dict) -> dict:
    """Feasibility verdict + capacity projections for one package."""
    bench, node = pkg["bench"], pkg["node"]
    ram_mb = node.get("ram_mb") or (bench.get("package") or {}).get("ram_mb")
    cpu = node.get("cpu_cores") or (bench.get("package") or {}).get("cpu_cores")
    storage_gb = node.get("storage_gb") or (bench.get("package") or {}).get("storage_gb")
    price = node.get("price_usd_mo") or (bench.get("package") or {}).get("price_usd_mo")

    sc = scenario_map(bench)
    full = sc.get(10_000)
    light = sc.get(1_000)
    empty = sc.get(0)

    checks = []
    ok = True

    # RAM: whole-scope peak (node + bench together)
    peak_kb = node.get("cgroup_peak_ram_kb")
    ram_used_pct = None
    if peak_kb and ram_mb:
        ram_used_pct = round(peak_kb / (ram_mb * 1024) * 100, 1)
        fits = ram_used_pct < 70
        ok &= fits
        checks.append({
            "check": "RAM fit (peak usage of full scope)",
            "result": "PASS" if fits else "FAIL",
            "detail": f"{peak_kb} KiB / {ram_mb} MB = {ram_used_pct}%",
        })

    # CPU: block production is single-threaded; 1 core must be enough.
    if cpu:
        fits = cpu >= 1
        ok &= fits
        checks.append({
            "check": "CPU cores for single-threaded block production",
            "result": "PASS" if fits else "FAIL",
            "detail": f"{cpu} core(s); bench peaks ~100% of one core",
        })

    # Latency: worst block (10k tx) must be far below default interval.
    latency_ok = None
    if full:
        latency_ok = full["median_ms"] < DEFAULT_BLOCK_INTERVAL_S * 1000 * 0.5
        ok &= latency_ok
        checks.append({
            "check": f"10k-tx block latency vs {DEFAULT_BLOCK_INTERVAL_S}s interval",
            "result": "PASS" if latency_ok else "FAIL",
            "detail": f"{full['median_ms']:.0f} ms << {DEFAULT_BLOCK_INTERVAL_S} s interval",
        })

    # Storage: daily chain growth (worst case = every block full 10k tx).
    daily_growth_gb = None
    daily_growth_full_gb = None
    if full and light:
        daily_growth_full_gb = round(
            full["block_bytes"] * SECONDS_PER_DAY / DEFAULT_BLOCK_INTERVAL_S / 1e9, 3)
        daily_growth_gb = round(
            light["block_bytes"] * SECONDS_PER_DAY / DEFAULT_BLOCK_INTERVAL_S / 1e9, 4)
        if storage_gb:
            days_full = storage_gb / daily_growth_full_gb if daily_growth_full_gb else None
            fits = days_full is None or days_full > 30
            checks.append({
                "check": "Storage: days to fill disk (worst case, every block 10k tx)",
                "result": "PASS" if fits else "WARN",
                "detail": (f"{days_full:.0f} days on {storage_gb} GB"
                           if days_full else "n/a"),
            })

    # Capacity: how many tx fit into the default block interval.
    capacity_per_interval = None
    if full and full["median_ms"] > 0:
        capacity_per_interval = int(
            DEFAULT_BLOCK_INTERVAL_S * 1000 / full["median_ms"] * 10_000)

    return {
        "package": pkg["name"],
        "cpu_cores": cpu,
        "ram_mb": ram_mb,
        "storage_gb": storage_gb,
        "price_usd_mo": price,
        "verdict": "SUPPORTED" if ok else "NOT RECOMMENDED",
        "checks": checks,
        "node": node,
        "bench_wall_ms": bench.get("scenarios") and node.get("bench_wall_ms"),
        "capacity_tx_per_interval": capacity_per_interval,
        "daily_chain_growth_gb_full_blocks": daily_growth_full_gb,
        "daily_chain_growth_gb_1k_blocks": daily_growth_gb,
        "scenarios": bench.get("scenarios", []),
        "total_cpu_time_ms": bench.get("total_cpu_time_ms"),
    }


def main() -> None:
    packages = load_packages()
    if not packages:
        raise SystemExit(f"no simulation results found in {SIM_DIR}")

    results = [evaluate(p) for p in packages]

    machine = {
        "host_os": platform.system(),
        "host_arch": platform.machine(),
        "host_cpu_count": os.cpu_count(),
        "host_total_ram_kb": next(
            (int(line.split()[1]) for line in open("/proc/meminfo")
             if line.startswith("MemTotal:")), None),
    }

    with open(OUT_JSON, "w") as f:
        json.dump({
            "generated_at": datetime.datetime.now(datetime.UTC).isoformat(),
            "source": "https://www.namecheap.com/hosting/vps/",
            "default_block_interval_s": DEFAULT_BLOCK_INTERVAL_S,
            "methodology": "cgroup v2 CPUQuota/MemoryMax per package",
            "host": machine,
            "notes": NOTES,
            "packages": results,
        }, f, indent=2)

    lines = []
    a = lines.append
    a("# Namecheap VPS Simulation Report — Miraset Chain")
    a("")
    a(f"- Generated at: {datetime.datetime.now(datetime.UTC).isoformat()}")
    a(f"- Source of specs/prices: https://www.namecheap.com/hosting/vps/")
    a(f"- Methodology: node + benchmark inside cgroup scope per package "
      f"(CPUQuota = package cores, MemoryMax = package RAM); default block "
      f"interval {DEFAULT_BLOCK_INTERVAL_S} s.")
    a(f"- Host: {machine['host_os']} {machine['host_arch']}, "
      f"{machine['host_cpu_count']} cores, "
      f"{fmt((machine['host_total_ram_kb'] or 0) / 1024 / 1024, ' GiB', 1)}")
    a("")
    a("## Summary")
    a("")
    a("| Package | CPU | RAM | Disk | $/mo | Verdict | Node peak RSS | Scope peak RAM | "
      "10k block | Max tx/interval |")
    a("|---|---:|---:|---:|---:|---|---:|---:|---:|---:|")
    for r in results:
        sc = scenario_map({"scenarios": r["scenarios"]})
        full = sc.get(10_000)
        a("| {pkg} | {cpu} | {ram} MB | {disk} GB | {price} | **{verdict}** | {nrss} | "
          "{cpeak} | {blk} | {cap} |".format(
              pkg=r["package"], cpu=fmt(r["cpu_cores"], "", 0),
              ram=fmt(r["ram_mb"], "", 0), disk=fmt(r["storage_gb"], "", 0),
              price=fmt(r["price_usd_mo"], "", 2), verdict=r["verdict"],
              nrss=fmt((r["node"].get("node_peak_rss_kb") or 0) / 1024, " MiB", 1),
              cpeak=fmt((r["node"].get("cgroup_peak_ram_kb") or 0) / 1024, " MiB", 1),
              blk=(f"{full['median_ms']:.0f} ms" if full else "-"),
              cap=f"{r['capacity_tx_per_interval']:,}"
              if r["capacity_tx_per_interval"] else "-"))
    a("")

    for r in results:
        a(f"## {r['package']} — {fmt(r['cpu_cores'], '', 0)} CPU, "
          f"{fmt(r['ram_mb'], '', 0)} MB RAM, {fmt(r['storage_gb'], '', 0)} GB, "
          f"${fmt(r['price_usd_mo'], '', 2)}/mo")
        a("")
        node = r["node"]
        a("### Node runtime (under package limits)")
        a("")
        a(f"- Node started: {'yes' if node.get('node_started') else 'NO'}")
        a(f"- Benchmark exit code: {node.get('bench_exit_code', 'n/a')}")
        a(f"- Blocks produced during run (5 s interval): {node.get('blocks_produced', 'n/a')} "
          f"(height {node.get('height_start', '?')} → {node.get('height_end', '?')})")
        a(f"- Node peak RSS: {fmt((node.get('node_peak_rss_kb') or 0) / 1024, ' MiB', 1)}")
        a(f"- Node CPU during bench: {fmt(node.get('node_cpu_percent'), '%', 1)} "
          f"({node.get('node_cpu_ms', '?')} ms)")
        a(f"- Whole-scope peak RAM (node+bench): "
          f"{fmt((node.get('cgroup_peak_ram_kb') or 0) / 1024, ' MiB', 1)}")
        a(f"- Whole-scope CPU during bench: {fmt((node.get('cgroup_cpu_ms') or 0) / 1000, ' s', 2)}")
        a(f"- Bench wall time: {fmt((node.get('bench_wall_ms') or 0) / 1000, ' s', 2)}")
        a(f"- Node storage footprint after run: "
          f"{fmt_bytes(node.get('node_storage_bytes'))}")
        a("")
        a("### Block production benchmark")
        a("")
        a("| txs/block | median ms | min ms | max ms | tx/s | CPU ms | CPU % | "
          "block bytes | peak RSS MiB |")
        a("|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
        for s in r["scenarios"]:
            a("| {n} | {med:.2f} | {mn:.2f} | {mx:.2f} | {tput:.0f} | {cpu} | "
              "{cpup} | {bb} | {rss} |".format(
                  n=s["tx_count"], med=s["median_ms"], mn=s["min_ms"],
                  mx=s["max_ms"], tput=s["throughput_tx_per_s"],
                  cpu=fmt(s.get("cpu_time_ms"), "", 1),
                  cpup=fmt(s.get("cpu_percent"), "", 1),
                  bb=s["block_bytes"],
                  rss=fmt((s.get("peak_rss_kb") or 0) / 1024, "", 1)))
        a("")
        a("### Projections & checks")
        a("")
        if r["capacity_tx_per_interval"]:
            a(f"- Sustainable throughput per {DEFAULT_BLOCK_INTERVAL_S} s block interval: "
              f"**{r['capacity_tx_per_interval']:,} tx** "
              f"(mempool cap is 10,000 tx — headroom "
              f"{r['capacity_tx_per_interval'] // 10_000}×)")
        if r["daily_chain_growth_gb_full_blocks"] is not None:
            a(f"- Chain growth, worst case (every block full, 10k tx): "
              f"{r['daily_chain_growth_gb_full_blocks']} GB/day")
            a(f"- Chain growth, light load (1k tx/block): "
              f"{r['daily_chain_growth_gb_1k_blocks']} GB/day")
        a("")
        a("| Check | Result | Detail |")
        a("|---|---|---|")
        for c in r["checks"]:
            a(f"| {c['check']} | {c['result']} | {c['detail']} |")
        a("")
        a(f"**Verdict: {r['verdict']}**")
        a("")

    a("## Assumptions & limitations")
    a("")
    for note in NOTES.strip().splitlines():
        note = note.strip().removeprefix("- ").strip()
        if note:
            a(f"- {note}")

    with open(OUT_MD, "w") as f:
        f.write("\n".join(lines) + "\n")

    print(f"wrote {OUT_MD}")
    print(f"wrote {OUT_JSON}")


if __name__ == "__main__":
    main()
