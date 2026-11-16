import json
import os
import subprocess
import sys

from checklib import check, finish

CASES = "/tmp/stats-cases"
os.makedirs(CASES, exist_ok=True)


def case(name, text):
    path = os.path.join(CASES, name)
    with open(path, "w") as f:
        f.write(text)
    return path


def run(tool, *args):
    done = subprocess.run([sys.executable, tool, *args], capture_output=True, text=True, timeout=30)
    return done.returncode, done.stdout, done.stderr


def same(*args):
    want = run("/tests/reference.py", *args)
    got = run("/app/stats.py", *args)
    assert got[0] == want[0], f"exit {got[0]}, wanted {want[0]}; stderr {got[2][-300:]!r}"
    assert got[1].strip() == want[1].strip(), f"stdout {got[1][-400:]!r}\nwanted {want[1][-400:]!r}"
    return True


def same_json(*args):
    want = json.loads(run("/tests/reference.py", *args)[1])
    got = json.loads(run("/app/stats.py", *args)[1])
    assert set(got) == set(want), sorted(got)
    for key in want:
        assert abs(got[key] - want[key]) < 1e-9, (key, got[key], want[key])
    return True


sample = "/app/data/sample.txt"
small = case("small.txt", "3\n1\n2\n10\n")
single = case("single.txt", "# one\n42\n")
empty = case("empty.txt", "# nothing\n\n")
bad = case("bad.txt", "1\n2\n# fine\nthree\n")

# The core: what a run that saves early should already have.
CORE = ["file exists", "sample statistics", "small file"]
check("file exists", lambda: os.path.exists("/app/stats.py"))
check("sample statistics", lambda: same(sample))
check("small file", lambda: same(small))
check("single number", lambda: same(single))
check("no data", lambda: same(empty))
check("not a number", lambda: same(bad))
check("json", lambda: same_json(sample, "--json"))
check("bins", lambda: same(sample, "--bins", "5"))
check("trim", lambda: same(sample, "--trim", "10"))
check("precision", lambda: same(small, "--precision", "2"))
finish(required=CORE)
