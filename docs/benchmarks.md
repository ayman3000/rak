# Benchmarks

How rak was measured, and against what. Back to the [README](../README.md).

`bench/` holds six small Python tasks (fix a bug, add a feature, write a CLI, change two files of a package, parse a log, fix code and write its tests). Each run happens in a fresh container that holds only rak and the task's starting files; tests the agent never sees are copied in afterwards and decide pass or fail.

## rak alone

```bash
docker build -f bench/Dockerfile -t rak-bench .
```

```bash
python3 bench/run.py --model kimi-k2.6:cloud --attempts 2
```

| Run | rak | Passed | Time | Tokens in (median per run) | Tokens out |
|---|---|---|---|---|---|
| 2026-11-10 | 0.1.0-alpha.1, before the chat and MCP | 12/12 | 530 s | 182K (13.6K) | 44K |
| 2026-11-11 | after the chat, MCP and the module split | 11/12 | 552 s | 203K (14.1K) | 47K |
| 2026-11-11 | with the write check | 12/12 | 434 s | 258K (19.3K), 222K of it cached | 41K |
| 2026-11-11 | with the honesty rule (first run) | 9/12 | 394 s | 185K (13.8K), 157K of it cached | 34K |
| 2026-11-11 | with the honesty rule (second run) | 12/12 | 521 s | 183K (15.5K), 158K of it cached | 53K |

All with kimi-k2.6:cloud through Ollama, two attempts per task. The tool descriptions were the same in every run; the system prompt too, until the last two runs added the honesty rule.

The honesty rule (report only checks that actually ran, and say what was not checked) was run twice because its first run scored 9/12. The three misses were the model's judgement, not missed checks: in each it had run its own tests. Task 3 reused a deleted to-do's number, task 4 refused the discount code `flat5`, and task 6 fell into the `"5 5"` trap. The second run scored 12/12. Together that is 21 of 24, in line with the runs before, with the same input per run.

The third run is the first that records cached tokens: 86% of the input was read from Ollama's cache. Ollama's cloud did that caching on its own; the earlier runs most likely had it too, unrecorded. The write check rejected no write in this run, so it caused no false alarms, but these tasks gave it nothing to catch either. Input was higher than in the earlier runs because the model took more turns; with this few attempts per task, that is within the run-to-run spread.

The miss was task 1: the model fixed the three bugs the visible tests catch and stopped, leaving the fourth thing the docstrings ask for (`restock` raising `KeyError` for an unknown item), which only the hidden test checks. Four more attempts at task 1 passed 3 of 4. Across all eight attempts that task passed 6 times, so the first run's 2/2 was luck rather than the second run's 1/2 a regression.

## Head to head with sak

`bench/head_to_head.py` runs rak and sak on the same six tasks with the same model on the same Mac, taking turns on every attempt. sak runs only on macOS, so neither runs in a container here. Instead each runs inside macOS's sandbox (`bench/sandbox.sb.in`): writes only to its copy of the task and its own temporary folders, network only to Ollama. Both run in auto mode with MCP off, and are graded by the same hidden tests as above. (These runs had no Tavily key for rak, so no `web_search`; with one, rak's fixed cost per call grows by that tool's description.)

```bash
python3 bench/head_to_head.py --model kimi-k2.6:cloud --attempts 3
```

2026-11-11, kimi-k2.6:cloud through Ollama, three attempts per task:

| | Passed | Model calls | Tokens in (median per run) | Tokens out | Time (median per run) |
|---|---|---|---|---|---|
| rak 0.1.0-alpha.1 | 17/18 | 161 | 385K (19.0K) | 75K | 724 s (20.8 s) |
| sak 0.3.0-alpha | 18/18 | 157 | 511K (22.0K) | 75K | 685 s (25.7 s) |

What this says:

- **rak carries sak's design over faithfully.** Both made almost the same number of model calls (161 against 157) and wrote the same amount (75K tokens each). The pass rates are within one attempt of each other. rak's miss was the model's judgement, not the harness: on task 6 it let `"5 5"` through where the docstring wanted an error.
- **rak sent 25% less input, and nearly all of that is a fixed cost per call, not smarter behaviour.** A request that uses no tools costs rak 1,037 tokens and sak 1,825. sak carries two more tools (`fetch_url`, and `web_search` because a Tavily key is set) and a longer prompt. That gap of 788 tokens, times sak's 157 calls, is 124K: almost all of the 126K difference.
- **Neither is faster.** Time is the model writing. rak's median run was shorter and its total longer, because of one attempt where the model wrote 24K tokens. Rust against Swift makes no difference that shows.

So rak is as good as sak at the same work. Its lower token count comes from carrying fewer tools by default, which sak could match by turning its web tools off. Whether this design beats other agents is a different test: against Hermes, OpenCode or Claude Code on the same open model.

## Head to head with Hermes

Hermes Agent runs its agent inside a background gateway process, which a sandbox around the command would not contain. So both agents run in the same Linux container image (`bench/Dockerfile.hermes`: rak's benchmark image plus Hermes, installed from its own lock file), as the same non-root user, reaching Ollama on the host. Hermes starts from a fresh home each run with only its Ollama provider set: its defaults, its own turn limit.

```bash
docker build -f bench/Dockerfile.hermes -t rak-hermes-bench ~/.hermes/hermes-agent
```

```bash
python3 bench/head_to_head.py --model kimi-k2.6:cloud --attempts 3 --against hermes
```

2026-11-11, kimi-k2.6:cloud through Ollama, three attempts per task:

| | Passed | Model calls | Tokens in (median per run) | Of that, from cache | Tokens out | Time (median per run) |
|---|---|---|---|---|---|---|
| rak 0.1.0-alpha.1 | 16/18 | 148 | 314K (15.5K) | 86% | 86K | 736 s (23.1 s) |
| Hermes Agent 0.22.4 (`81a5543`) | 16/18 | 131 | 1,660K (96.8K) | 96% | 50K | 595 s (28.4 s) |

What this says:

- **The same results.** Both passed 16 of 18. Three of the four misses were the same one: task 6's `"5 5"`, which the docstring calls invalid (rak once, Hermes twice). rak's other miss was task 1's `KeyError`, met before.
- **Hermes sent 5.3 times as much input, and most of it is a fixed cost per call.** A request that uses no tools costs Hermes 10,354 input tokens and rak 990: Hermes's system prompt and its much larger set of tools. That gap of 9,364 tokens, times Hermes's 131 calls, is 1.23M: 91% of the 1.35M difference.
- **Caching narrows the gap a lot.** Hermes's fixed part is the same on every call, so 96% of its input came from the server's cache. Input not from the cache was 60K for Hermes and 43K for rak, 1.4 times as much. What that means in money depends on how the server prices cached input. Where it costs a tenth of normal input, as on Anthropic's API, Hermes's input costs about three times rak's. On a server that does not cache, five times.
- **Neither is clearly faster.** Hermes finished the whole run sooner (595 s against 736 s), but only because of task 6, where the model wrote far more through rak. Two of Hermes's three attempts there failed. Without task 6, rak took 371 s and Hermes 410 s. On the same task and attempt, rak was faster in 11 of 18, and each won three tasks on median time. rak is quicker per token written (8.6 s per 1,000 against 11.8 s), which fits Hermes sending five times the input. Hermes made fewer calls (131 against 148).
- **Hermes also made small helper calls** (titles), 2K tokens in all, counted apart above.

**Memory and CPU** (`bench/resources.py`: fresh containers of the same image, measured inside each; the model runs on Ollama's servers, so this is the agent's own cost plus the commands it runs; medians):

| | One-word reply: memory · CPU · time | Task 2: memory · CPU · time | Installed |
|---|---|---|---|
| rak | 4.0 MB · 0.10 s · 1.3 s | 6.0 MB · 0.79 s · 17.2 s | one 2.6 MB file |
| Hermes | 357 MB · 3.48 s · 6.2 s | 374 MB · 6.71 s · 31.8 s | 292 MB, plus its own 92 MB Python |

Memory is the container's peak, which counts every process (Hermes's gateway too) and the files they read. The sampled total of the processes' own memory gave the same picture: 12 MB for rak, most of it the measuring shell itself, against 386–423 MB for Hermes. Hermes needs about 5 seconds and 3.5 seconds of CPU to start before it sends anything to the model.

So, on these tasks with an open model, rak reaches the same results as Hermes with about a fifth of the input, around a sixtieth of the memory and an eighth of the CPU. Neither is clearly faster to finish. The fixed cost per call is rak's lead on tokens. The next test is harder tasks, where more calls make that lead count for more, and where Hermes's larger toolset might help.

These tasks are easy: a perfect score says the pipeline works, not how rak ranks. They are not the tasks sak was measured on, so the numbers do not compare with sak's.
