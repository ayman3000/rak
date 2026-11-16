# rak on Harbor (Terminal-Bench 2.1, DeepSWE)

[`rak_agent.py`](rak_agent.py) runs rak as a [Harbor](https://harborframework.com) installed agent: it installs a pinned rak release in each task's container and runs one unattended request (`rak -y --json --base-url URL -m MODEL "<task>"`). This is how the [Harness Leaderboard](https://huggingface.co/spaces/joelniklaus/harness-leaderboard) runs harnesses: same model for all, reached through a base URL.

Status: written for Harbor 0.26 and run on 10 Terminal-Bench 2.1 tasks (2026-11-14, see [Trial](#trial-2026-11-14)).

## What you need

- Docker running. On a Mac, [OrbStack](https://orbstack.dev) works (`brew install --cask orbstack`, then pick Docker in its welcome screen).
- Harbor: `uv tool install harbor`.
- A model endpoint that speaks Chat Completions. Ollama on the host works: the container reaches it at `http://host.docker.internal:11434/v1`, with no key. For a provider that needs one, export `RAK_API_KEY` in the shell that runs Harbor.
- A rak release: the adapter installs v0.1.0-alpha.16 (long shell output keeps its end since alpha.11; the task's time limit is passed to rak since alpha.16).

## Running a few Terminal-Bench 2.1 tasks

```bash
cd bench/harbor
PYTHONPATH=. harbor run \
  -d terminal-bench/terminal-bench-2-1 \
  -a rak_agent:RakAgent \
  -m deepseek-v4.1-flash:cloud \
  --base-url http://host.docker.internal:11434/v1 \
  --effort low \
  -l 10 -k 1 -n 2 \
  -o ~/rak-harbor-jobs -y -q
```

- `-l 10` takes ten tasks, `-k 1` one attempt each, `-n 2` two at a time.
- `-i '*name*'` / `-x '*name*'` pick or skip tasks; they are globs, so a bare task name matches nothing.
- Options: `--effort`, and `--context` / `--max-tokens` through the agent's options (see `harbor run --help`).
- Each trial's `agent/` folder holds `rak-result.json` (the `--json` summary), `rak-log.jsonl` (every step) and `rak-progress.txt` (empty under `--json`). A run stopped at the time limit has no summary; its tokens are added up from the step log.
- Each task pulls its own image (about 0.3–1 GB). `docker system prune -a` afterwards gives the space back.

## Trial (2026-11-14)

rak 0.1.0-alpha.12, `deepseek-v4.1-flash:cloud` through Ollama, effort low, one attempt, on an Apple Silicon Mac (the task images are x86, so they run emulated).

| Task | Reward | Note |
|---|---|---|
| write-compressor | 1 | 8.6 min, 43 turns (a later re-run timed out at 15 min) |
| kv-store-grpc | 1 | 45 s, 14 turns |
| schemelike-metacircular-eval | 1 | Hit the 40-min limit while making the tests run with the evaluator interpreting itself (part of the task); an alpha.13 re-run did the same |
| openssl-selfsigned-cert | 1 | 22 s, 12 turns |
| pypi-server | 1 | 66 s, 20 turns |
| regex-chess | 1 | Still testing at the 60-min limit |
| dna-assembly | 0 | Finished with a wrong answer |
| qemu-alpine-ssh | 0 | Timed out booting a VM under QEMU inside the emulated container |
| torch-tensor-parallelism | – | rak finished; the grader timed out |
| torch-pipeline-parallelism | – | rak finished; the grader timed out |

Six of ten. What it changed: the adapter installs with wget when the image has it (qemu-alpine-ssh's Debian 11 image cannot install curl). The timeouts most likely come from a cheap model at low effort on emulated x86 (not checked on x86); a full run belongs on an x86 Linux machine.
