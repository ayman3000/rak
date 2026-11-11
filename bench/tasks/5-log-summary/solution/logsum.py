import json
import re
import sys
from collections import Counter

LINE = re.compile(r'^(\S+) - - \[[^\]]+\] "(\S+) (\S+) (\S+)" (\d{3}) (\d+|-)$')


def summarize(lines):
    statuses, paths, ips = Counter(), Counter(), set()
    requests = size = skipped = 0
    for line in lines:
        line = line.strip()
        if not line:
            continue
        m = LINE.match(line)
        if not m:
            skipped += 1
            continue
        requests += 1
        ips.add(m.group(1))
        paths[m.group(3)] += 1
        statuses[m.group(5)] += 1
        size += 0 if m.group(6) == "-" else int(m.group(6))
    top = sorted(paths.items(), key=lambda kv: (-kv[1], kv[0]))[:3]
    return {"requests": requests, "bytes": size, "statuses": dict(statuses),
            "top_paths": [[p, c] for p, c in top], "unique_ips": len(ips), "skipped": skipped}


if __name__ == "__main__":
    try:
        with open(sys.argv[1]) as f:
            print(json.dumps(summarize(f)))
    except OSError:
        print(f"logsum: cannot read {sys.argv[1]}", file=sys.stderr)
        sys.exit(2)
