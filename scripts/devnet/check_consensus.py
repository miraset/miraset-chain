#!/usr/bin/env python3
"""Check a running miraset devnet for agreement (safety) and progress (liveness).

Safety: every reachable node reports the same committed block hash at every height up to the
lowest committed height among them. Liveness (optional): the highest committed height among
reachable nodes grows by at least --min-progress blocks within --progress-window seconds.
Convergence (optional): within --progress-window, every reachable node comes within --max-lag
blocks of the highest one (catches a recovered node that never caught up).

Exit codes: 0 ok, 1 conflicting finality, 2 no progress / too few reachable nodes.
Uses only the Python standard library.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request

DEFAULT_RPCS = ",".join(f"http://127.0.0.1:{port}" for port in range(8545, 8550))
REQUEST_TIMEOUT_SECS = 2


def get_json(url: str) -> dict | None:
    try:
        with urllib.request.urlopen(url, timeout=REQUEST_TIMEOUT_SECS) as resp:
            return json.load(resp)
    except (urllib.error.URLError, OSError, ValueError):
        return None


def committed_height(rpc: str) -> int | None:
    body = get_json(f"{rpc}/v1/blocks/latest")
    if not body or not body.get("success"):
        return None
    return body["data"].get("height")


def hash_at(rpc: str, height: int) -> str | None:
    body = get_json(f"{rpc}/v1/blocks/{height}")
    if not body or not body.get("success"):
        return None
    return body["data"]["hash"]


def heights(rpcs: list[str]) -> dict[str, int | None]:
    return {rpc: committed_height(rpc) for rpc in rpcs}


def check_safety(
    rpcs: list[str], max_heights: int
) -> tuple[tuple[int, dict[str, str | None]] | bool | None, dict[str, int]]:
    """Compare hashes at each shared height (the most recent `max_heights` if set)."""
    reachable = {rpc: h for rpc, h in heights(rpcs).items() if h is not None}
    if len(reachable) < 2:
        return None, reachable
    shared = min(reachable.values())
    start = 0 if max_heights <= 0 else max(0, shared - max_heights + 1)
    for height in range(start, shared + 1):
        hashes = {rpc: hash_at(rpc, height) for rpc in reachable}
        distinct = {h for h in hashes.values() if h is not None}
        if len(distinct) > 1:
            return (height, hashes), reachable
    return False, reachable


def wait_for_progress(
    rpcs: list[str], window: float, min_progress: int
) -> tuple[int | None, int | None]:
    def best() -> int | None:
        values = [h for h in heights(rpcs).values() if h is not None]
        return max(values) if values else None

    start = best()
    deadline = time.monotonic() + window
    while time.monotonic() < deadline:
        current = best()
        if start is not None and current is not None and current - start >= min_progress:
            return start, current
        if start is None:
            start = current
        time.sleep(0.5)
    return start, best()


def wait_for_convergence(rpcs: list[str], window: float, max_lag: int) -> dict[str, int | None]:
    """Return {} once all nodes are reachable and within `max_lag`, else the laggards."""
    deadline = time.monotonic() + window
    while True:
        current = heights(rpcs)
        known = [h for h in current.values() if h is not None]
        top = max(known) if known else None
        laggards = {
            rpc: h for rpc, h in current.items() if h is None or top is None or top - h > max_lag
        }
        if not laggards or time.monotonic() >= deadline:
            return laggards
        time.sleep(0.5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--rpc", default=DEFAULT_RPCS, help="comma-separated RPC base URLs")
    parser.add_argument("--min-reachable", type=int, default=2)
    parser.add_argument("--max-heights", type=int, default=0, help="0 = check every height")
    parser.add_argument("--progress-window", type=float, default=0, help="seconds; 0 = skip")
    parser.add_argument("--min-progress", type=int, default=1)
    parser.add_argument("--max-lag", type=int, default=-1, help="blocks; -1 = skip")
    args = parser.parse_args()
    rpcs = [r.strip().rstrip("/") for r in args.rpc.split(",") if r.strip()]

    if args.progress_window > 0:
        start, end = wait_for_progress(rpcs, args.progress_window, args.min_progress)
        print(f"progress: committed height {start} -> {end} in <= {args.progress_window}s")
        if start is None or end is None or end - start < args.min_progress:
            print("FAIL: no consensus progress", file=sys.stderr)
            return 2

    if args.max_lag >= 0:
        window = args.progress_window if args.progress_window > 0 else 30
        laggards = wait_for_convergence(rpcs, window, args.max_lag)
        if laggards:
            print(f"FAIL: nodes not within {args.max_lag} blocks: {laggards}", file=sys.stderr)
            return 2

    conflict, reachable = check_safety(rpcs, args.max_heights)
    for rpc in rpcs:
        print(f"  {rpc}: height={reachable.get(rpc, 'unreachable')}")
    if len(reachable) < args.min_reachable:
        print(f"FAIL: only {len(reachable)} reachable nodes", file=sys.stderr)
        return 2
    if conflict:
        height, hashes = conflict
        print(f"FAIL: conflicting finality at height {height}: {hashes}", file=sys.stderr)
        return 1
    print(f"OK: {len(reachable)} nodes agree up to height {min(reachable.values())}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
