# rak probes

Five small Harbor tasks of our own, each aimed at one behaviour, for comparing
rak setups without tuning rak on the FrontierHarness tasks themselves.

| Task | Measures | Limit |
|---|---|---|
| `config-keys` | Batching: a 15-file survey; model calls per task, tool calls per call | 15 min |
| `invoice-cli` | Near misses: 14 hidden checks of small, exact requirements | 15 min |
| `pager-fix` | Stopping: model calls made after the tests first pass | 15 min |
| `stats-timed` | Time awareness: a long spec under 6 minutes; the reward needs only the core | 6 min |
| `sliding-puzzle` | Thinking: an optimal 8-puzzle solution (31 moves); output tokens and time | 15 min |

Each task has a reference solution (`solution/solve.sh`) and a checker
(`tests/check.py`) that writes the reward and a `ctrf.json` with every check,
so a near miss shows how close it came. All five pass with Harbor's `oracle`
agent and fail with `nop` (checked on Runta, 2026-11-16).

- `config-keys/generate.py` writes that task's files and expected answer.
- `sync.sh` copies `_shared/checklib.py` into each task's `tests/`.
- `~/rak-frontier/run-probes.sh SETUP` runs all five, 2 attempts each, in one
  setup: `a15`, `a16`, `a16-low` (`--effort low`) or `a17`.
- `report.py` compares the setups: passes, checks, model calls, tool calls
  per call, tokens, minutes, todo use, time warnings, and calls after green.

Runta wants at least 32 GB of disk per task (`storage_mb = 32768`).
