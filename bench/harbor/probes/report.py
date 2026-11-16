"""Compares rak setups on the probe tasks, from Harbor's job folders.

    python3 report.py [JOBS_DIR]     (default ~/rak-frontier/probe-jobs)

JOBS_DIR holds one folder per setup (a15, a16, ...), each with Harbor's job
folders inside. For each setup and task, over its attempts: how many passed,
the checks passed, model calls, tool calls per model call, output and input
tokens, minutes, todo use, time warnings (70% and 90%), and, on pager-fix, the model calls made
after the tests first passed (stopping).
"""

import json
import statistics
import sys
from pathlib import Path


def trial_stats(trial: Path) -> dict | None:
    result_file = trial / "result.json"
    if not result_file.exists():
        return None
    result = json.loads(result_file.read_text())
    reward = ((result.get("verifier_result") or {}).get("rewards") or {}).get("reward")
    error = (result.get("exception_info") or {}).get("exception_type", "")
    checks = ""
    ctrf = trial / "verifier" / "ctrf.json"
    if ctrf.exists():
        summary = json.loads(ctrf.read_text())["results"]["summary"]
        checks = f"{summary['passed']}/{summary['tests']}"
    calls = tools = todo = notes = out = inp = 0
    first = last = None
    green_at = None
    log = trial / "agent" / "rak-log.jsonl"
    if log.exists():
        for line in log.read_text().splitlines():
            try:
                event = json.loads(line)
            except ValueError:
                continue
            kind = event.get("event")
            if event.get("time"):
                first = first or event["time"]
                last = event["time"]
            if kind == "model_call":
                calls += 1
                out += event.get("output_tokens") or 0
                inp += event.get("input_tokens") or 0
            elif kind == "tool_call":
                tools += 1
                if event.get("tool") == "todo":
                    todo += 1
            elif kind == "note" and "time is running out" in (event.get("note") or ""):
                notes += 1
            elif kind == "tool_result" and green_at is None:
                text = str(event.get("result", ""))
                if " passed" in text and " failed" not in text and "error" not in text.lower():
                    green_at = calls
    return {
        "pass": reward == 1,
        "error": error,
        "checks": checks,
        "calls": calls,
        "per_call": tools / calls if calls else 0,
        "out": out,
        "in": inp,
        "minutes": (last - first) / 60 if first and last else 0,
        "todo": todo,
        "notes": notes,
        "after_green": calls - green_at if green_at is not None else None,
    }


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else Path.home() / "rak-frontier" / "probe-jobs")
    rows = []
    for setup in sorted(p for p in root.iterdir() if p.is_dir() and not p.name.startswith("20")):
        by_task: dict[str, list[dict]] = {}
        for trial in setup.glob("*/*__*"):
            stats = trial_stats(trial)
            if stats:
                by_task.setdefault(trial.name.split("__")[0], []).append(stats)
        for task, runs in sorted(by_task.items()):
            mean = lambda key: statistics.mean(r[key] for r in runs)
            greens = [r["after_green"] for r in runs if r["after_green"] is not None]
            rows.append([
                setup.name,
                task,
                f"{sum(r['pass'] for r in runs)}/{len(runs)}",
                " ".join(r["checks"] or "-" for r in runs),
                f"{mean('calls'):.0f}",
                f"{mean('per_call'):.2f}",
                f"{mean('out') / 1000:.1f}K",
                f"{mean('in') / 1e6:.2f}M",
                f"{mean('minutes'):.1f}",
                f"{mean('todo'):.1f}",
                f"{mean('notes'):.1f}",
                f"{statistics.mean(greens):.1f}" if greens else "-",
                " ".join(sorted({r["error"] for r in runs if r["error"]})) or "",
            ])
    header = ["setup", "task", "passed", "checks", "calls", "tools/call", "out", "in", "min", "todo", "time notes", "after green", "errors"]
    widths = [max(len(str(x)) for x in col) for col in zip(header, *rows)] if rows else [len(h) for h in header]
    for row in [header, *rows]:
        print("  ".join(str(cell).ljust(width) for cell, width in zip(row, widths)))


if __name__ == "__main__":
    main()
