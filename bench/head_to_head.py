#!/usr/bin/env python3
"""rak against another agent on the same tasks, model and machine.

Both agents run in auto mode, take turns on every attempt (so both meet the
model server in the same conditions), and are graded the same way as
bench/run.py: the hidden tests are copied in afterwards and run in the
rak-bench container, with no network.

Against sak: sak runs only on macOS, so both agents run on this Mac, each in
macOS's sandbox (bench/sandbox.sb.in). Writes go only to the task's copy and
the run's own temp folders; the only network is Ollama on 127.0.0.1. MCP is
off for both.

  python3 bench/head_to_head.py --model kimi-k2.6:cloud --attempts 3

Against Hermes: Hermes runs its agent inside a background gateway process,
which a sandbox around the command would not contain. So both agents run in
the same Linux container image instead (bench/Dockerfile.hermes), as the same
non-root user, reaching Ollama on the host. Hermes starts from a fresh home
each run with only its model provider set: its defaults, its own turn limit.

  docker build -f bench/Dockerfile -t rak-bench .
  docker build -f bench/Dockerfile.hermes -t rak-hermes-bench ~/.hermes/hermes-agent
  python3 bench/head_to_head.py --model kimi-k2.6:cloud --attempts 3 --against hermes
"""
import argparse, json, os, pathlib, shutil, statistics, subprocess, sys, time

HERE = pathlib.Path(__file__).resolve().parent
OLLAMA = "http://127.0.0.1:11434"
OLLAMA_FROM_CONTAINER = "http://host.docker.internal:11434"


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--attempts", type=int, default=3)
    p.add_argument("--only", help="run only tasks whose name contains this")
    p.add_argument("--timeout", type=int, default=900, help="seconds allowed per attempt")
    p.add_argument("--max-turns", type=int, default=30, help="for rak and sak; Hermes keeps its own limit")
    p.add_argument("--image", default="rak-bench", help="the container the hidden tests run in")
    p.add_argument("--agents-image", default="rak-hermes-bench", help="the container both agents run in against Hermes")
    p.add_argument("--against", default="sak", choices=["sak", "hermes"], help="the agent rak is compared with")
    a = p.parse_args()

    agents = ("rak", a.against)
    versions = {agent: version(agent, a) for agent in agents}
    out = HERE / "results" / (f"h2h-{a.against}-" + time.strftime("%Y%m%d-%H%M%S"))
    (out / "logs").mkdir(parents=True)
    tasks = sorted(t for t in (HERE / "tasks").iterdir() if t.is_dir() and (not a.only or a.only in t.name))
    rows = []
    for task in tasks:
        for attempt in range(1, a.attempts + 1):
            # Alternate who goes first, so neither always meets a warmer server.
            order = agents if attempt % 2 else agents[::-1]
            for agent in order:
                row = run_attempt(agent, task, attempt, a, out)
                rows.append(row)
                print(f"{agent:<6} {'PASS' if row['passed'] else 'FAIL'}  {row['task']:<18} #{attempt}  {row['outcome']:<9} "
                      f"{row['turns']:>2} calls  {row['input_tokens']:>7} in  {row['output_tokens']:>5} out  "
                      f"{row['seconds']:>6.1f} s", flush=True)

    summary = {"model": a.model, "attempts": a.attempts, "against": a.against, "versions": versions, "agents": {}, "rows": rows}
    print()
    for agent in agents:
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
            "helper_tokens": sum(r.get("helper_tokens", 0) for r in mine),
        }
        summary["agents"][agent] = totals
        print(f"{agent:<6} {versions[agent][:28]:<28} {totals['passed']}/{totals['runs']} passed · {totals['seconds']} s "
              f"(median {totals['median_seconds']} s) · {totals['turns']} model calls · {totals['input_tokens']} in "
              f"(median {totals['median_input_tokens']}) · {totals['output_tokens']} out")
    (out / "results.json").write_text(json.dumps(summary, indent=2))
    print(out / "results.json")


def version(agent, a):
    command = [agent, "--version"]
    if a.against == "hermes":
        command = ["docker", "run", "--rm", "-w", "/opt/hermes", a.agents_image] + command
    text = subprocess.run(command, capture_output=True, text=True).stdout.strip()
    return text.splitlines()[0] if text else "?"


# MARK: - On this Mac, in macOS's sandbox (rak against sak)

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
    """The command for this Mac (rak and sak)."""
    if agent == "rak":
        return ["rak", "-y", "--json", "--no-mcp", "--provider", "openai", "--base-url", f"{OLLAMA}/v1",
                "-m", a.model, "--max-turns", str(a.max_turns), task_text]
    return ["sak", "-y", "--json", "-P", "ollama", "-m", a.model, "--turns", str(a.max_turns), task_text]


def run_on_mac(agent, a, task_text, work, tmp, run_dir):
    (run_dir / "sandbox.sb").write_text(sandbox_profile(work, tmp))
    # TMPPREFIX: zsh puts here-documents there (default /tmp, which is closed).
    env = dict(os.environ, TMPDIR=str(tmp), TMPPREFIX=str(tmp / "zsh"), OLLAMA_HOST=OLLAMA, SAK_NASEEM="0", NO_COLOR="1")
    for key in ("RAK_MODEL", "RAK_BASE_URL", "RAK_PROVIDER", "SAK_PROVIDER", "SAK_MODEL", "SAK_AUTO"):
        env.pop(key, None)
    command = ["sandbox-exec", "-f", str(run_dir / "sandbox.sb")] + command_for(agent, a, task_text)
    return subprocess.run(command, cwd=work, env=env, capture_output=True, text=True, timeout=a.timeout, stdin=subprocess.DEVNULL)


# MARK: - In a Linux container (rak against Hermes)

def run_in_container(agent, a, task_text, work, tmp, name):
    # The container's user is not the Mac's: these throwaway copies must be writable by it.
    for folder in (work, tmp):
        subprocess.run(["chmod", "-R", "a+rwX", str(folder)], check=True)
    if agent == "rak":
        inner = ["rak", "-y", "--json", "--no-mcp", "--provider", "openai", "--base-url", f"{OLLAMA_FROM_CONTAINER}/v1",
                 "-m", a.model, "--max-turns", str(a.max_turns), task_text]
    else:
        inner = ["hermes", "-z", task_text, "-m", a.model, "--usage-file", "/out/usage.json"]
    container = f"h2h-{name}-{os.getpid()}"
    # --init reaps the child processes an agent leaves behind (Hermes warns without it).
    command = ["docker", "run", "--rm", "--init", "--name", container, "--add-host=host.docker.internal:host-gateway",
               "-v", f"{work}:/work", "-v", f"{tmp}:/out", "-w", "/work", "-e", "NO_COLOR=1", a.agents_image] + inner
    try:
        return subprocess.run(command, capture_output=True, text=True, timeout=a.timeout, stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        subprocess.run(["docker", "kill", container], capture_output=True)
        raise


# MARK: - Results

def read_result(agent, stdout, tmp):
    """The agent's own report → the fields every agent shares."""
    if agent == "hermes":
        # -z prints only the answer; the numbers are in its usage report. Its
        # helper calls (titles, summaries) are counted apart from the agent's.
        usage = json.loads((tmp / "usage.json").read_text())
        outcome = "done" if usage.get("completed") and not usage.get("failed") else ("failed" if usage.get("failed") else "stopped")
        return {"turns": usage.get("api_calls", 0), "tool_calls": 0,
                "input_tokens": usage.get("input_tokens", 0), "cached_tokens": usage.get("cache_read_tokens", 0),
                "output_tokens": usage.get("output_tokens", 0), "helper_tokens": usage.get("auxiliary", {}).get("total_tokens", 0),
                "outcome": outcome, "answer": stdout, "usage": usage}
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
    task_text = (task / "task.md").read_text().strip()

    row = {"agent": agent, "task": task.name, "attempt": attempt, "outcome": "timeout", "turns": 0, "tool_calls": 0,
           "input_tokens": 0, "cached_tokens": 0, "output_tokens": 0, "seconds": 0.0}
    started = time.time()
    try:
        if a.against == "hermes":
            r = run_in_container(agent, a, task_text, work, tmp, name)
        else:
            r = run_on_mac(agent, a, task_text, work, tmp, run_dir)
        (out / "logs" / f"{name}.stderr.txt").write_text(r.stderr)
        try:
            row.update(read_result(agent, r.stdout, tmp))
            if "usage" in row:
                (out / "logs" / f"{name}.usage.json").write_text(json.dumps(row.pop("usage"), indent=2))
        except (ValueError, OSError):
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
