#!/bin/bash
cat > /app/stats.py <<'PYEOF'
"""The reference stats tool, for the checker and the solution."""

import json
import math
import sys


def percentile(values, q):
    ordered = sorted(values)
    pos = (len(ordered) - 1) * q
    low = math.floor(pos)
    high = min(low + 1, len(ordered) - 1)
    return ordered[low] + (ordered[high] - ordered[low]) * (pos - low)


def main(argv):
    args = argv[1:]
    path, as_json, bins, trim, digits = None, False, None, 0.0, 4
    while args:
        arg = args.pop(0)
        if arg == "--json":
            as_json = True
        elif arg == "--bins":
            bins = int(args.pop(0))
        elif arg == "--trim":
            trim = float(args.pop(0))
        elif arg == "--precision":
            digits = int(args.pop(0))
        else:
            path = arg
    values = []
    for number, line in enumerate(open(path), start=1):
        text = line.strip()
        if not text or text.startswith("#"):
            continue
        try:
            values.append(float(text))
        except ValueError:
            print(f"error: line {number}: not a number", file=sys.stderr)
            return 3
    if trim:
        cut = int(len(values) * trim / 100)
        values = sorted(values)[cut:len(values) - cut] if cut else values
    if not values:
        print("no data")
        return 0
    n = len(values)
    mean = sum(values) / n
    stats = {
        "count": n,
        "sum": sum(values),
        "mean": mean,
        "median": percentile(values, 0.5),
        "stdev": math.sqrt(sum((v - mean) ** 2 for v in values) / (n - 1)) if n > 1 else 0.0,
        "min": min(values),
        "max": max(values),
        "p90": percentile(values, 0.9),
    }
    if as_json:
        print(json.dumps(stats))
    else:
        for name, value in stats.items():
            print(f"{name}: {value}" if name == "count" else f"{name}: {value:.{digits}f}")
    if bins:
        low, high = stats["min"], stats["max"]
        width = (high - low) / bins
        counts = [0] * bins
        for v in values:
            index = min(int((v - low) / width), bins - 1) if width else 0
            counts[index] += 1
        for i, count in enumerate(counts):
            lo, hi = low + i * width, low + (i + 1) * width
            close = "]" if i == bins - 1 else ")"
            print(f"[{lo:.2f}, {hi:.2f}{close} {count}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
PYEOF
