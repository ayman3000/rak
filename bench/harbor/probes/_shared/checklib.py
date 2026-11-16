"""Shared by the probes' checkers: run named checks, then write Harbor's
reward (1 when every required check passes) and a ctrf.json with each check,
so a near miss shows how close it came. Copied into each task's tests/ by
probes/sync.sh."""

import json
import os

RESULTS = []


def check(name, fn):
    try:
        ok, note = bool(fn()), ""
    except Exception as error:  # a crash is a failed check, with its reason
        ok, note = False, repr(error)[:300]
    RESULTS.append((name, ok, note))
    print(("PASS " if ok else "FAIL ") + name + (f"  ({note})" if note else ""))
    return ok


def finish(required=None):
    os.makedirs("/logs/verifier", exist_ok=True)
    passed = sum(ok for _, ok, _ in RESULTS)
    report = {
        "results": {
            "summary": {"tests": len(RESULTS), "passed": passed, "failed": len(RESULTS) - passed},
            "tests": [{"name": n, "status": "passed" if ok else "failed", "message": m} for n, ok, m in RESULTS],
        }
    }
    with open("/logs/verifier/ctrf.json", "w") as f:
        json.dump(report, f, indent=1)
    needed = [r for r in RESULTS if required is None or r[0] in required]
    reward = 1 if needed and all(ok for _, ok, _ in needed) else 0
    with open("/logs/verifier/reward.txt", "w") as f:
        f.write(f"{reward}\n")
    print(f"{passed}/{len(RESULTS)} checks passed; reward {reward}")
