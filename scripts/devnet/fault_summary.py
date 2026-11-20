"""Per-fault report for scripts/devnet/fault_load.sh (TODO 8.5.3, measurement plan §4.3).

Reads OUT_DIR/faults.log (fault_suite.sh output, each line prefixed with epoch seconds),
OUT_DIR/heights.txt (epoch seconds, then committed height of node0..node4; -1 = unreachable) and
OUT_DIR/load.json (loadgen result). For every fault_suite step it prints:
- the longest gap between commits of the highest node during the step (the quorum's commit gap),
- for a step that names a node: that node's lag behind the highest node at the start, and the
  time until it is within CAUGHT_UP_LAG blocks (recovery steps), or "-" if it never was.
Usage: python3 -I fault_summary.py OUT_DIR
"""

import json
import re
import sys

CAUGHT_UP_LAG = 3


def load_heights(path):
    rows = []
    for line in open(path):
        parts = line.split()
        if len(parts) != 6:
            continue
        try:
            rows.append((float(parts[0]), [int(x) for x in parts[1:]]))
        except ValueError:
            continue
    return rows


def load_steps(path):
    steps = []
    for line in open(path, errors="replace"):
        m = re.match(r"^(\d+\.\d+) === (.*) ===$", line.rstrip("\n"))
        if m:
            steps.append((float(m.group(1)), m.group(2)))
    return steps


def commit_gap(rows, start, end):
    """Longest time the highest committed height did not grow within [start, end)."""
    window = [(t, max(h)) for t, h in rows if start <= t < end]
    longest, last_t, last_h = 0.0, None, None
    for t, h in window:
        if last_h is None or h > last_h:
            if last_t is not None:
                longest = max(longest, t - last_t)
            last_t, last_h = t, h
    if last_t is not None and window:
        longest = max(longest, window[-1][0] - last_t)
    return longest


def catch_up(rows, start, end, node):
    """(lag at start, seconds until within CAUGHT_UP_LAG, or None)."""
    window = [(t, h) for t, h in rows if start <= t < end]
    if not window:
        return None, None
    first = window[0][1]
    lag0 = max(first) - first[node] if first[node] >= 0 else None
    for t, h in window:
        if h[node] >= 0 and max(h) - h[node] <= CAUGHT_UP_LAG:
            return lag0, t - start
    return lag0, None


def main(out):
    rows = load_heights(f"{out}/heights.txt")
    steps = load_steps(f"{out}/faults.log")
    if not rows or not steps:
        print("no heights or no fault steps recorded")
        return 1
    ends = [t for t, _ in steps[1:]] + [rows[-1][0]]
    print(f"{'step':<72} {'secs':>5} {'gap s':>6} {'lag0':>5} {'caught s':>8}")
    for (start, name), end in zip(steps, ends):
        node = re.search(r"node(\d)", name)
        lag0 = caught = None
        if node and name.split(":")[0] in ("recover", "heal"):
            lag0, caught = catch_up(rows, start, end, int(node.group(1)))
        fmt = lambda v: "-" if v is None else (f"{v:.0f}" if isinstance(v, float) else str(v))
        print(f"{name[:72]:<72} {end - start:5.0f} {commit_gap(rows, start, end):6.1f} "
              f"{fmt(lag0):>5} {fmt(caught):>8}")
    try:
        load = json.load(open(f"{out}/load.json"))
        print(f"\nload: offered {load.get('offered_rate')}/s, committed {load.get('committed_per_sec')}/s, "
              f"pending at end {load.get('pending_at_end')}, rejected {load.get('rejected')}")
    except (OSError, ValueError) as error:
        print(f"\nload: no result ({error})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
