#!/usr/bin/env python3
"""Runs rak on every task in bench/tasks and grades the result with tests the
agent never sees.

Each attempt runs in a fresh container that holds only rak and a copy of the
task's starting files. The hidden tests are copied in after rak has exited and
run in a second container with no network.

  docker build -f bench/Dockerfile -t rak-bench .
  python3 bench/run.py --model kimi-k2.6:cloud
"""
import argparse, json, pathlib, shutil, subprocess, sys, time

HERE = pathlib.Path(__file__).resolve().parent


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--base-url", default="http://host.docker.internal:11434/v1")
    p.add_argument("--api-key-env", help="name of an environment variable to pass into the container as RAK_API_KEY")
    p.add_argument("--effort")
    p.add_argument("--attempts", type=int, default=1)
    p.add_argument("--only", help="run only tasks whose name contains this")
    p.add_argument("--timeout", type=int, default=900, help="seconds allowed per attempt")
    p.add_argument("--image", default="rak-bench")
    p.add_argument("--max-turns", type=int, default=30)
    a = p.parse_args()

    out = HERE / "results" / time.strftime("%Y%m%d-%H%M%S")
    out.mkdir(parents=True)
    tasks = sorted(t for t in (HERE / "tasks").iterdir() if t.is_dir() and (not a.only or a.only in t.name))
    rows = []
    for task in tasks:
        for attempt in range(1, a.attempts + 1):
            row = run_attempt(task, attempt, a, out)
            rows.append(row)
            print(f"{'PASS' if row['passed'] else 'FAIL'}  {row['task']:<18} #{attempt}  {row['outcome']:<9} "
                  f"{row['turns']:>2} turns  {row['tool_calls']:>2} calls  {row['input_tokens']:>7} in  "
                  f"{row['output_tokens']:>5} out  {row['seconds']:>6.1f} s", flush=True)

    passed = sum(r["passed"] for r in rows)
    summary = {
        "model": a.model, "base_url": a.base_url, "effort": a.effort, "attempts": a.attempts,
        "rak": subprocess.run(["docker", "run", "--rm", a.image, "rak", "--version"], capture_output=True, text=True).stdout.strip(),
        "passed": passed, "runs": len(rows),
        "seconds": round(sum(r["seconds"] for r in rows), 1),
        "input_tokens": sum(r["input_tokens"] for r in rows),
        "output_tokens": sum(r["output_tokens"] for r in rows),
        "rows": rows,
    }
    (out / "results.json").write_text(json.dumps(summary, indent=2))
    print(f"\n{passed}/{len(rows)} passed · {summary['seconds']} s · {summary['input_tokens']} tokens in · "
          f"{summary['output_tokens']} out\n{out / 'results.json'}")


def run_attempt(task, attempt, a, out):
    name = f"{task.name}-{attempt}"
    work = out / "work" / name
    shutil.copytree(task / "start", work)
    logs = out / "logs"
    logs.mkdir(exist_ok=True)

    command = ["docker", "run", "--rm", "--add-host=host.docker.internal:host-gateway",
               "-v", f"{work}:/work", "-v", f"{logs}:/logs"]
    if a.api_key_env:
        command += ["-e", f"RAK_API_KEY={__import__('os').environ[a.api_key_env]}"]
    command += [a.image, "rak", "-y", "--json", "--log", f"/logs/{name}.jsonl", "-m", a.model, "--base-url", a.base_url,
                "--max-turns", str(a.max_turns), "-C", "/work"]
    if a.effort:
        command += ["--effort", a.effort]
    command.append((task / "task.md").read_text().strip())

    started = time.time()
    row = {"task": task.name, "attempt": attempt, "outcome": "timeout", "turns": 0, "tool_calls": 0,
           "input_tokens": 0, "output_tokens": 0, "seconds": 0.0, "error": None}
    try:
        r = subprocess.run(command, capture_output=True, text=True, timeout=a.timeout)
        try:
            row.update({k: v for k, v in json.loads(r.stdout.strip().splitlines()[-1]).items() if k in row or k == "answer"})
        except (ValueError, IndexError):
            row.update(outcome="crashed", error=(r.stderr or r.stdout)[-400:])
    except subprocess.TimeoutExpired:
        pass
    row["seconds"] = round(time.time() - started, 1)
    (logs / f"{name}.answer.md").write_text(row.pop("answer", "") or "")

    # Grade: the hidden tests go in only now, and run without network.
    shutil.copytree(task / "hidden", work, dirs_exist_ok=True)
    try:
        graded = subprocess.run(["docker", "run", "--rm", "--network", "none", "-v", f"{work}:/work", a.image,
                                 "python3", "-m", "unittest", "discover", "-s", "/work", "-p", "hidden_test*.py"],
                                capture_output=True, text=True, timeout=180)
        row["passed"] = graded.returncode == 0
        (logs / f"{name}.grade.txt").write_text(graded.stderr + graded.stdout)
    except subprocess.TimeoutExpired:
        row["passed"] = False
    return row


if __name__ == "__main__":
    sys.exit(main())
