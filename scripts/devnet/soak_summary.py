#!/usr/bin/env python3
"""Summarise a soak run directory (samples.csv + load.json) as Markdown.

Per validator: resident memory at the start and end of the steady phase and its least-squares
growth rate, file descriptor range, chain data growth rate, height progress and restarts. The
first 10% of samples are treated as warm-up and excluded from growth rates.

Usage: scripts/devnet/soak_summary.py <soak-dir>
Standard library only.
"""

from __future__ import annotations

import csv
import json
import pathlib
import sys
from collections import defaultdict

MIB = 1024 * 1024
HOUR = 3600.0


def slope_per_hour(points: list[tuple[float, float]]) -> float | None:
    """Least-squares slope of (seconds, value) points, in value units per hour."""
    if len(points) < 2:
        return None
    n = len(points)
    mean_t = sum(t for t, _ in points) / n
    mean_v = sum(v for _, v in points) / n
    var = sum((t - mean_t) ** 2 for t, _ in points)
    if var == 0:
        return None
    cov = sum((t - mean_t) * (v - mean_v) for t, v in points)
    return cov / var * HOUR


def column(rows: list[dict], name: str) -> list[tuple[float, float]]:
    return [(float(r["ts"]), float(r[name])) for r in rows if r.get(name)]


def fmt(value: float | None, scale: float = 1.0, digits: int = 1) -> str:
    return "-" if value is None else f"{value / scale:.{digits}f}"


def node_row(node: str, rows: list[dict]) -> list[str]:
    steady = rows[len(rows) // 10:] or rows
    rss = column(steady, "rss_bytes")
    data = column(steady, "data_bytes")
    fds = [v for _, v in column(rows, "fds")]
    heights = [v for _, v in column(rows, "height")]
    restarts = [v for _, v in column(rows, "restarts")]
    return [
        node,
        fmt(rss[0][1] if rss else None, MIB),
        fmt(rss[-1][1] if rss else None, MIB),
        fmt(slope_per_hour(rss), MIB, 2),
        f"{int(min(fds))}-{int(max(fds))}" if fds else "-",
        fmt(slope_per_hour(data), MIB, 2),
        f"{int(heights[0])}->{int(heights[-1])}" if heights else "-",
        fmt(max(restarts) - min(restarts) if restarts else None, 1.0, 0),
    ]


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 64
    directory = pathlib.Path(sys.argv[1])
    by_node: dict[str, list[dict]] = defaultdict(list)
    with (directory / "samples.csv").open() as handle:
        for row in csv.DictReader(handle):
            by_node[row["node"]].append(row)

    header = ["node", "RSS start MiB", "RSS end MiB", "RSS MiB/h", "fds", "data MiB/h",
              "height", "restarts"]
    print("| " + " | ".join(header) + " |")
    print("|" + "---|" * len(header))
    for node in sorted(by_node):
        print("| " + " | ".join(node_row(node, by_node[node])) + " |")

    load_path = directory / "load.json"
    try:
        report = json.loads(load_path.read_text())
    except (OSError, ValueError) as error:
        print(f"\nno load report ({error})")
        return 0
    tx, rc = report["transactions"], report["receipts"]
    print(
        f"\nload: offered {report['offered_rate']}/s for {report['duration_secs']:.0f}s, "
        f"committed {report['committed_per_sec']}/s; tx p50/p95/p99 "
        f"{tx['latency']['p50_ms']}/{tx['latency']['p95_ms']}/{tx['latency']['p99_ms']} ms; "
        f"receipt p50/p95 {rc['latency']['p50_ms']}/{rc['latency']['p95_ms']} ms; "
        f"rejected {sum(tx['rejected'].values()) + sum(rc['rejected'].values())}; "
        f"uncommitted {tx['uncommitted'] + rc['uncommitted']}; pending at end {report['pending_at_end']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
