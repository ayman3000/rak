# rak — Rust Agent Kit

A small coding agent for the terminal. One binary, no runtime to install, for Linux and macOS (Windows is written but untested).

rak is the cross-platform sibling of [sak](https://github.com/ayman3000/sak). sak is built on SwiftAgentKit and stays the Mac tool; rak carries the same prompt, the same tool designs and the same loop to everything else.

> **Status: early alpha (0.1.0-alpha.1).** The core works and is tested. Much of what sak has is not here yet — see [What is missing](#what-is-missing).

## What it does

You give it a task. It reads your project, edits files, runs commands, and tells you what it did.

```
$ rak -m kimi-k2.6:cloud "Make the tests pass."
→ list_dir path=.
  ✓ calc.py test_calc.py
→ read_file path=calc.py
→ read_file path=test_calc.py
→ run_shell python3 -m unittest test_calc -v
? allow run_shell python3 -m unittest test_calc -v
  [y]es  [n]o  [a]lways for run_shell  [A] everything this session › y
  ✓ exit 1 test_average ... ok test_average_of_nothing_is_zero ... ERROR …
→ edit_file calc.py  (7 → 9 lines)
  ✓ Replaced 1 occurrence in calc.py.
→ run_shell python3 -m unittest test_calc -v
  ✓ exit 0 …
All 3 tests pass. I fixed two bugs in `calc.py`: …
  8 turns · 8 tool calls · 12127 tokens in · 653 out
```

## Install

You need [Rust](https://rustup.rs) 1.87 or newer.

```bash
cargo install --path .
```

## Use

rak talks to any server that speaks OpenAI's chat-completions API: OpenAI, OpenRouter, Ollama, Hugging Face's router, vLLM, llama.cpp, LM Studio.

```bash
# Ollama on this machine (the default address)
rak -m kimi-k2.6:cloud "explain what this project does"

# Any other server
export RAK_BASE_URL=https://openrouter.ai/api/v1
export RAK_API_KEY=...
rak -m some/model "add a --verbose flag to the CLI"

# No task: a plain chat, one request per line
rak -m kimi-k2.6:cloud
```

| Option | What it does |
|---|---|
| `-m`, `--model NAME` | Model to use. Or set `RAK_MODEL`. |
| `--base-url URL` | Server address. Or set `RAK_BASE_URL`. Default `http://localhost:11434/v1`. |
| `--api-key-env NAME` | Environment variable holding the key. Default `RAK_API_KEY`, then `OPENAI_API_KEY`. |
| `--effort LEVEL` | Reasoning effort: `low`, `medium` or `high`. |
| `--max-tokens N` | Longest reply to ask for. |
| `--context N` | The model's context window in tokens. Default 128000. |
| `-y`, `--yes` | Run every tool without asking. |
| `--read-only` | No edits and no commands: read and report. |
| `--any-path` | Let file tools reach outside the project folder. |
| `-C`, `--cwd DIR` | The project folder. |
| `--max-turns N` | Model calls allowed per request. Default 30. |
| `--json` | Print one JSON object: answer, outcome, turns, tokens, seconds. |
| `--log FILE` | Append every step to FILE as JSON lines. |
| `-q`, `--quiet` | Only the answer. |

Exit codes: `0` done, `1` failed, `2` bad usage, `3` a tool was not approved, `4` turn limit, `5` stuck repeating a call.

## Safety

- **Edits and commands ask first.** `write_file`, `edit_file` and `run_shell` need a yes each time, unless you answer "always" or start with `-y`.
- **A no stops the request.** The model does not get another turn to try the same change a different way.
- **No terminal, no changes.** In a script or CI without `-y`, rak stops with exit code 3 instead of guessing.
- **File tools stay in the project folder** unless you pass `--any-path`. Symlinks are resolved before the check.
- **`run_shell` is not confined.** A command you approve can do anything your user can. Read it before you say yes.

## Tools

| Tool | What it does |
|---|---|
| `list_dir` | Lists a folder. |
| `search_files` | Finds files by name and by text inside them. Skips `.git`, `node_modules`, `target`, `.build`. |
| `read_file` | Reads a file. Files up to 40,000 characters come back in one call; small `limit`s are widened, because many small reads cost more than one large one. |
| `write_file` | Writes or appends a file. Refuses to overwrite a file that changed on disk since rak last read it. |
| `edit_file` | Replaces exact text. Tolerates trailing-whitespace differences; when the text is not found it shows the file's lines near the closest match. |
| `run_shell` | Runs a command with a time limit that kills the whole process tree. Background mode for servers. |

## As a library

```rust
use rak::agent::{system_prompt, Agent, Config};
use rak::provider::OpenAICompatible;
use rak::tools::{core_tools, Ctx};

let cwd = std::env::current_dir()?;
let ctx = Ctx::new(&cwd, false);
let model = OpenAICompatible::new("http://localhost:11434/v1", None, "kimi-k2.6:cloud");
let config = Config { system_prompt: system_prompt(&ctx.cwd, false, None), max_turns: 30, context_window: 128_000 };
let mut agent = Agent::new(Box::new(model), core_tools(&ctx, false), config);
let result = agent.run("Make the tests pass.");
println!("{}", result.answer);
```

A runnable version is in [examples/minimal.rs](examples/minimal.rs). Add your own tool by implementing `rak::tools::Tool`; plug in another model API by implementing `rak::provider::Model`.

## What is missing

Compared with sak, rak does not yet have:

- streaming output, Markdown rendering, the welcome panel, the `/` command menu, line editing and history
- saved sessions (`-c` to continue)
- MCP servers, web fetch and web search
- native Anthropic and Gemini APIs (use an OpenAI-compatible endpoint)
- Naseem notes and skills
- a syntax check on written files
- Ctrl-C handling: stopping rak mid-command can leave that command running

Not yet tested: Windows, and any public benchmark. Auto mode (`-y`) has only been run inside the benchmark's containers.

## Development

```bash
cargo test
```

```bash
cargo build --release
```

MIT licence.

## Benchmark

`bench/` holds six small Python tasks (fix a bug, add a feature, write a CLI, change two files of a package, parse a log, fix code and write its tests). Each run happens in a fresh container that holds only rak and the task's starting files; tests the agent never sees are copied in afterwards and decide pass or fail.

```bash
docker build -f bench/Dockerfile -t rak-bench .
```

```bash
python3 bench/run.py --model kimi-k2.6:cloud --attempts 2
```

First run, 2026-11-10, rak 0.1.0-alpha.1 with kimi-k2.6:cloud through Ollama, two attempts per task: **12/12 passed**, 530 s in total, 182K tokens in (median 13.6K per run), 44K out.

These tasks are easy: a perfect score says the pipeline works, not how rak ranks. They are not the tasks sak was measured on, so the numbers do not compare with sak's.
