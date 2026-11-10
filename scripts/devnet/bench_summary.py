#!/usr/bin/env python3
"""Summarise `loadgen` JSON reports in a bench results directory as a Markdown table.

Usage: scripts/devnet/bench_summary.py <results-dir>
Standard library only.
"""

from __future__ import annotations

import json
import pathlib
import sys


def fmt(value: object) -> str:
    if value is None:
        return "-"
    if isinstance(value, float):
        return f"{value:g}"
    return str(value)


def rejected(kind: dict) -> int:
    return sum(kind["rejected"].values())


def row(report: dict) -> list[str]:
    tx, rc, blocks = report["transactions"], report["receipts"], report["blocks"]
    return [
        fmt(report["offered_rate"]),
        fmt(report["committed_per_sec"]),
        fmt(tx["latency"]["p50_ms"]),
        fmt(tx["latency"]["p95_ms"]),
        fmt(tx["latency"]["p99_ms"]),
        fmt(rc["latency"]["p50_ms"]),
        fmt(rc["latency"]["p95_ms"]),
        f"{rejected(tx) + rejected(rc)}",
        f"{tx['uncommitted'] + rc['uncommitted']}",
        fmt(blocks["mean_interval_ms"]),
        fmt(blocks["mean_txs_per_block"]),
        fmt(report["max_generator_lag_ms"]),
        fmt(report["max_observer_round_ms"]),
        fmt(report.get("observer_reconnects")),
    ]


HEADER = [
    "offered/s", "committed/s", "tx p50 ms", "tx p95 ms", "tx p99 ms", "rcpt p50 ms",
    "rcpt p95 ms", "rejected", "uncommitted", "block interval ms", "txs/block",
    "gen lag ms", "observer round ms", "reconnects",
]


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 64
    directory = pathlib.Path(sys.argv[1])
    reports = []
    for path in directory.glob("rate-*.json"):
        try:
            reports.append(json.loads(path.read_text()))
        except (OSError, ValueError) as error:
            print(f"skipping {path}: {error}", file=sys.stderr)
    reports.sort(key=lambda r: r["offered_rate"])
    print("| " + " | ".join(HEADER) + " |")
    print("|" + "---|" * len(HEADER))
    for report in reports:
        print("| " + " | ".join(row(report)) + " |")
    return 0


if __name__ == "__main__":
    sys.exit(main())
