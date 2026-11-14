# rak on Harbor (Terminal-Bench 2.1, DeepSWE)

[`rak_agent.py`](rak_agent.py) runs rak as a [Harbor](https://harborframework.com) installed agent: it installs a pinned rak release in each task's container and runs one unattended request (`rak -y --json --base-url URL -m MODEL "<task>"`). This is how the [Harness Leaderboard](https://huggingface.co/spaces/joelniklaus/harness-leaderboard) runs harnesses: same model for all, reached through a base URL.

Status: written against Harbor's `BaseInstalledAgent` (install/run), **not run yet**.

## What you need

- Docker running (or another runtime Harbor supports locally: podman, apple-container).
- Harbor: `uv tool install harbor`.
- A model endpoint that speaks Chat Completions. Ollama on this Mac works for a trial: the container reaches it at `http://host.docker.internal:11434/v1`, with no key. For a provider that needs one, export `RAK_API_KEY` in the shell that runs Harbor.
- A rak release that has the unattended-run fixes (no turn limit, keeps nothing, reasoning sent back): v0.1.0-alpha.10 or later.

## A small trial: one attempt at a few Terminal-Bench 2.1 tasks

```bash
cd bench/harbor
PYTHONPATH=. harbor run \
  -d terminal-bench/terminal-bench-2-1 \
  -a rak_agent:RakAgent \
  -m deepseek-v4.1-flash:cloud \
  --base-url http://host.docker.internal:11434/v1 \
  --ak effort=low \
  -e docker \
  -l 10 -k 1 -n 2 \
  -o ../../../rak-harbor-jobs
```

`-l 10` takes ten tasks, `-k 1` one attempt each, `-n 2` two at a time. Each trial's folder holds `rak-result.json` (the `--json` summary), `rak-progress.txt` (what rak showed) and `rak-log.jsonl` (every step).

Agent options (`--ak key=value`): `version` (a rak release tag), `context` (`--context`), `max_tokens` (`--max-tokens`), `effort` (`--effort`).

Check `harbor run --help` first: flag names may differ between Harbor versions (`-a module:Class` against `--agent-import-path`).
