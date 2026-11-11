#!/usr/bin/env python3
"""Memory and CPU of rak and Hermes in the same container image.

Each run is a fresh container (rak-hermes-bench). Measured inside it:
- peak memory of the whole container (cgroup memory.peak; counts every
  process, Hermes's gateway included, and file cache),
- peak of the processes' summed resident memory (RSS), sampled 5 times a second,
- CPU time of the whole container (cgroup cpu.stat). The model runs on the
  Ollama server, so this is the agent's own work plus the commands it runs.

  python3 bench/resources.py --model kimi-k2.6:cloud
"""
import argparse, json, pathlib, shutil, statistics, subprocess, time

HERE = pathlib.Path(__file__).resolve().parent
ONE_WORD = "Reply with only the word: ok. Do not use any tools."

SCRIPT = r'''
( while :; do s=$(ps -eo rss= | awk '{s+=$1} END {print s+0}'); [ "$s" -gt "$(cat /tmp/rss 2>/dev/null || echo 0)" ] && echo "$s" > /tmp/rss; sleep 0.2; done ) &
sampler=$!
start=$(date +%s.%N)
"$@" > /out/stdout.txt 2> /out/stderr.txt
code=$?
end=$(date +%s.%N)
kill $sampler 2>/dev/null
printf '{"exit": %s, "start": %s, "end": %s, "memory_peak_bytes": %s, "rss_peak_kb": %s, "cpu_usec": %s}\n' \
  "$code" "$start" "$end" "$(cat /sys/fs/cgroup/memory.peak)" "$(cat /tmp/rss 2>/dev/null || echo 0)" \
  "$(awk '/^usage_usec/ {print $2}' /sys/fs/cgroup/cpu.stat)" > /out/resources.json
'''


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--model", required=True)
    p.add_argument("--image", default="rak-hermes-bench")
    p.add_argument("--one-word-runs", type=int, default=3)
    p.add_argument("--task", default="2-top-words")
    p.add_argument("--task-runs", type=int, default=2)
    a = p.parse_args()
    out = HERE / "results" / ("resources-" + time.strftime("%Y%m%d-%H%M%S"))
    out.mkdir(parents=True)
    task = HERE / "tasks" / a.task
    results = []
    jobs = [("one word", ONE_WORD, None)] * a.one_word_runs + [(a.task, (task / "task.md").read_text().strip(), task)] * a.task_runs
    for n, (job, text, task_dir) in enumerate(jobs):
        for agent in ("rak", "hermes") if n % 2 == 0 else ("hermes", "rak"):
            row = run(agent, job, text, task_dir, a, out / f"{n}-{agent}")
            results.append(row)
            print(f"{agent:<6} {job:<12} exit {row['exit']}  {row['seconds']:6.1f} s  peak {row['memory_peak_bytes'] / 2**20:7.1f} MB  "
                  f"RSS {row['rss_peak_kb'] / 1024:7.1f} MB  CPU {row['cpu_usec'] / 1e6:6.2f} s", flush=True)
    (out / "results.json").write_text(json.dumps(results, indent=2))
    print()
    for job in dict.fromkeys(r["job"] for r in results):
        for agent in ("rak", "hermes"):
            mine = [r for r in results if r["job"] == job and r["agent"] == agent]
            med = lambda k: statistics.median(r[k] for r in mine)
            print(f"{job:<12} {agent:<6} median: peak {med('memory_peak_bytes') / 2**20:7.1f} MB · RSS {med('rss_peak_kb') / 1024:7.1f} MB · "
                  f"CPU {med('cpu_usec') / 1e6:6.2f} s · {med('seconds'):6.1f} s")
    print(out / "results.json")


def run(agent, job, text, task_dir, a, run_dir):
    work, logs = run_dir / "work", run_dir / "out"
    if task_dir:
        shutil.copytree(task_dir / "start", work)
    else:
        work.mkdir(parents=True)
    logs.mkdir(parents=True)
    subprocess.run(["chmod", "-R", "a+rwX", str(run_dir)], check=True)
    if agent == "rak":
        inner = ["rak", "-y", "--json", "--no-mcp", "--provider", "openai", "--base-url", "http://host.docker.internal:11434/v1", "-m", a.model, text]
    else:
        inner = ["hermes", "-z", text, "-m", a.model]
    subprocess.run(["docker", "run", "--rm", "--init", "--add-host=host.docker.internal:host-gateway", "-v", f"{work}:/work",
                    "-v", f"{logs}:/out", "-w", "/work", a.image, "sh", "-c", SCRIPT, "sh"] + inner, check=True, timeout=900)
    measured = json.loads((logs / "resources.json").read_text())
    measured["seconds"] = round(measured.pop("end") - measured.pop("start"), 1)
    return {"agent": agent, "job": job} | measured


if __name__ == "__main__":
    main()
