#!/usr/bin/env python3
"""rak against sak on the same tasks, model and machine.

sak runs only on macOS, so this runs both agents on this Mac rather than in
containers, each inside macOS's sandbox (bench/sandbox.sb.in): it may write
only to its copy of the task and its own temp folders, and reach only Ollama
on 127.0.0.1. Both run in auto mode (-y) with their MCP servers off, and the
two agents take turns on each attempt, so both meet the model server in the
same conditions. Grading is the same as bench/run.py: the hidden tests are
copied in afterwards and run in the rak-bench container, with no network.

  docker build -f bench/Dockerfile -t rak-bench .
  python3 bench/head_to_head.py --model kimi-k2.6:cloud --attempts 3
"""
import argparse, json, os, pathlib, shutil, statistics, subprocess, sys, time

HERE = pathlib.Path(__file__).resolve().parent
OLLAMA = "http://127.0.0.1:11434"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--attempts", type=int, default=3)
    p.add_argument("--only", help="run only tasks whose name contains this")
    p.add_argument("--timeout", type=int, default=900, help="seconds allowed per attempt")
    p.add_argument("--max-turns", type=int, default=30)
    p.add_argument("--image", default="rak-bench", help="the container the hidden tests run in")
    a = p.parse_args()

    versions = {agent: subprocess.run([agent, "--version"], capture_output=True, text=True).stdout.strip() for agent in ("rak", "sak")}
    out = HERE / "results" / ("h2h-" + time.strftime("%Y%m%d-%H%M%S"))
    (out / "logs").mkdir(parents=True)
    tasks = sorted(t for t in (HERE / "tasks").iterdir() if t.is_dir() and (not a.only or a.only in t.name))
    rows = []
    for task in tasks:
        for attempt in range(1, a.attempts + 1):
            # Alternate who goes first, so neither always meets a warmer server.
            order = ("rak", "sak") if attempt % 2 else ("sak", "rak")
            for agent in order:
                row = run_attempt(agent, task, attempt, a, out)
                rows.append(row)
                print(f"{agent}  {'PASS' if row['passed'] else 'FAIL'}  {row['task']:<18} #{attempt}  {row['outcome']:<9} "
                      f"{row['turns']:>2} turns  {row['tool_calls']:>2} calls  {row['input_tokens']:>7} in  "
                      f"{row['output_tokens']:>5} out  {row['seconds']:>6.1f} s", flush=True)

    summary = {"model": a.model, "attempts": a.attempts, "versions": versions, "agents": {}, "rows": rows}
    print()
    for agent in ("rak", "sak"):
        mine = [r for r in rows if r["agent"] == agent]
        totals = {
            "passed": sum(r["passed"] for r in mine), "runs": len(mine),
            "seconds": round(sum(r["seconds"] for r in mine), 1),
            "median_seconds": round(statistics.median(r["seconds"] for r in mine), 1),
            "input_tokens": sum(r["input_tokens"] for r in mine),
            "median_input_tokens": statistics.median(r["input_tokens"] for r in mine),
            "cached_tokens": sum(r["cached_tokens"] for r in mine),
            "output_tokens": sum(r["output_tokens"] for r in mine),
            "turns": sum(r["turns"] for r in mine),
        }
        summary["agents"][agent] = totals
        print(f"{agent}  {versions[agent]:<20} {totals['passed']}/{totals['runs']} passed · {totals['seconds']} s "
              f"(median {totals['median_seconds']} s) · {totals['input_tokens']} in (median {totals['median_input_tokens']}) · "
              f"{totals['output_tokens']} out")
    (out / "results.json").write_text(json.dumps(summary, indent=2))
    print(out / "results.json")


def sandbox_profile(work, tmp):
    def conf(name):
        # The sandbox matches real paths: /var/folders/... is /private/var/folders/...
        return os.path.realpath(subprocess.run(["getconf", name], capture_output=True, text=True).stdout.strip())
    text = (HERE / "sandbox.sb.in").read_text()
    for key, value in {"@WORK@": work, "@TMP@": tmp, "@DARWIN_TEMP@": conf("DARWIN_USER_TEMP_DIR"),
                       "@DARWIN_CACHE@": conf("DARWIN_USER_CACHE_DIR")}.items():
        text = text.replace(key, str(value))
    return text


def command_for(agent, a, task_text):
    if agent == "rak":
        return ["rak", "-y", "--json", "--no-mcp", "--provider", "openai", "--base-url", f"{OLLAMA}/v1",
                "-m", a.model, "--max-turns", str(a.max_turns), task_text]
    return ["sak", "-y", "--json", "-P", "ollama", "-m", a.model, "--turns", str(a.max_turns), task_text]


def read_result(agent, stdout):
    """The agent's JSON result → the fields both share."""
    start = stdout.find("{")
    data = json.loads(stdout[start:]) if start >= 0 else {}
    if agent == "rak":
        return {k: data.get(k, 0) for k in ("turns", "tool_calls", "input_tokens", "cached_tokens", "output_tokens")} | {
            "outcome": data.get("outcome", "?"), "answer": data.get("answer", "")}
    usage = data.get("usage", {})
    codes = {0: "done", 1: "failed", 3: "refused", 4: "max_turns", 130: "stopped"}
    return {"turns": data.get("turns", 0), "tool_calls": data.get("tool_calls", 0),
            "input_tokens": usage.get("input_tokens", 0), "cached_tokens": usage.get("cached_tokens", 0),
            "output_tokens": usage.get("output_tokens", 0),
            "outcome": codes.get(data.get("exit_code"), f"exit {data.get('exit_code')}"), "answer": data.get("response", "")}


def run_attempt(agent, task, attempt, a, out):
    name = f"{agent}-{task.name}-{attempt}"
    run_dir = out / "work" / name
    work, tmp = run_dir / "project", run_dir / "tmp"
    shutil.copytree(task / "start", work)
    tmp.mkdir()
    profile = run_dir / "sandbox.sb"
    profile.write_text(sandbox_profile(work, tmp))

    # TMPPREFIX: zsh puts here-documents there (default /tmp, which is closed).
    env = dict(os.environ, TMPDIR=str(tmp), TMPPREFIX=str(tmp / "zsh"), OLLAMA_HOST=OLLAMA, SAK_NASEEM="0", NO_COLOR="1")
    for key in ("RAK_MODEL", "RAK_BASE_URL", "RAK_PROVIDER", "SAK_PROVIDER", "SAK_MODEL", "SAK_AUTO"):
        env.pop(key, None)
    command = ["sandbox-exec", "-f", str(profile)] + command_for(agent, a, (task / "task.md").read_text().strip())

    row = {"agent": agent, "task": task.name, "attempt": attempt, "outcome": "timeout", "turns": 0, "tool_calls": 0,
           "input_tokens": 0, "cached_tokens": 0, "output_tokens": 0, "seconds": 0.0}
    started = time.time()
    try:
        r = subprocess.run(command, cwd=work, env=env, capture_output=True, text=True, timeout=a.timeout, stdin=subprocess.DEVNULL)
        (out / "logs" / f"{name}.stderr.txt").write_text(r.stderr)
        try:
            row.update(read_result(agent, r.stdout))
        except ValueError:
            row.update(outcome="crashed")
            (out / "logs" / f"{name}.stdout.txt").write_text(r.stdout)
    except subprocess.TimeoutExpired:
        pass
    row["seconds"] = round(time.time() - started, 1)
    (out / "logs" / f"{name}.answer.md").write_text(row.pop("answer", "") or "")

    # Grade exactly as bench/run.py does.
    shutil.copytree(task / "hidden", work, dirs_exist_ok=True)
    try:
        graded = subprocess.run(["docker", "run", "--rm", "--network", "none", "-v", f"{work}:/work", a.image,
                                 "python3", "-m", "unittest", "discover", "-s", "/work", "-p", "hidden_test*.py"],
                                capture_output=True, text=True, timeout=180)
        row["passed"] = graded.returncode == 0
        (out / "logs" / f"{name}.grade.txt").write_text(graded.stderr + graded.stdout)
    except subprocess.TimeoutExpired:
        row["passed"] = False
    return row


if __name__ == "__main__":
    sys.exit(main())
