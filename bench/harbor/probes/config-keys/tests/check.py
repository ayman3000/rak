import json

from checklib import check, finish

expected = json.load(open("/tests/expected.json"))
rows = {}
order = []


def parse():
    lines = [l.strip() for l in open("/app/KEYS.md") if l.strip().startswith("|")]
    header = [c.strip() for c in lines[0].strip("|").split("|")]
    assert header == ["key", "default", "files"], header
    for line in lines[2:]:
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        rows[cells[0]] = cells
        order.append(cells[0])
    return True


check("KEYS.md exists with the right header", parse)
check("no extra keys", lambda: set(rows) <= set(expected))
check("sorted by key", lambda: order == sorted(order) and len(order) > 0)
for key, want in expected.items():
    check(f"row {key}", lambda key=key, want=want: rows[key][1] == want["default"] and rows[key][2] == ", ".join(want["files"]))
finish()
