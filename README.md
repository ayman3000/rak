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

# No task: a chat
rak -m kimi-k2.6:cloud
```

## Chat

Started without a task, rak opens a chat with the same interface as sak:

```
╭─ rak 0.1.0-alpha.1 ───────────────────────────────────────────────────────────────────────╮
│ ██████   █████  ██   ██            │ Tools                                                │
│ ██   ██ ██   ██ ██  ██             │ files  list_dir, search_files, read_file,            │
│ ██████  ███████ █████              │        edit_file, write_file                         │
│ ██   ██ ██   ██ ██  ██             │ shell  run_shell                                     │
│ ██   ██ ██   ██ ██   ██            │                                                      │
│ a coding agent for your terminal   │                                                      │
│                                    │                                                      │
│ model   kimi-k2.6:cloud            │                                                      │
│ via     localhost:11434            │                                                      │
│ folder  ~/projects/shop            │                                                      │
│ branch  main                       │                                                      │
│ mode    asks before changes        │                                                      │
│ window  262K tokens                │                                                      │
│                                                                                           │
│ 6 tools · /help for commands · Ctrl-C stops a request · Ctrl-D quits                      │
╰───────────────────────────────────────────────────────────────────────────────────────────╯

── kimi-k2.6:cloud · ctx 1.8K/262K ▰▱▱▱▱▱▱▱▱▱ 1% · shop (main) · asks before changes ────────
❯ /mo
❯ /model [name]  show or switch the model (no name: pick from a list)
  /models        list the models the server offers
```

- **The rule above the prompt** shows the model, how full its context is, the folder and branch, and the mode.
- **Type `/`** for the commands. The list narrows as you type; ↑ and ↓ choose, Tab completes, Enter runs, Esc closes it.
- **Answers appear as they are written**, with Markdown shown as headings, bullets and code rather than as symbols.
- **Ctrl-C stops a request** — the answer being written, or a command that is running — and keeps the chat. A second Ctrl-C quits.
- **History** is kept between runs (↑ and ↓). Pasted text keeps its line breaks; a line ending in `\` continues on the next.
- **No model named?** rak uses the one you last picked with `/model`, or lists the server's models to pick from.

| Command | What it does |
|---|---|
| `/model [name]` | Show or switch the model. Without a name: pick from a list. |
| `/models` | List the models the server offers. |
| `/tools` | List the tools the agent can use. |
| `/mcp` | List the MCP servers and their tools. |
| `/notes` | Show the project notes rak loaded (`AGENTS.md`). |
| `/yes` | Turn auto-approve on or off. |
| `/usage` | Tokens and tool calls so far, and the context window. |
| `/clear` | Start a fresh conversation. |
| `/help`, `/exit` | The list of commands; quit. |

The history and the last model picked are kept in `~/.config/rak/`.

| Option | What it does |
|---|---|
| `-m`, `--model NAME` | Model to use. Or set `RAK_MODEL`. A chat can start without one. |
| `--base-url URL` | Server address. Or set `RAK_BASE_URL`. Default `http://localhost:11434/v1`. |
| `--api-key-env NAME` | Environment variable holding the key. Default `RAK_API_KEY`, then `OPENAI_API_KEY`. |
| `--effort LEVEL` | Reasoning effort: `low`, `medium` or `high`. |
| `--max-tokens N` | Longest reply to ask for. |
| `--context N` | The model's context window in tokens. Default: what an Ollama server reports for the model, otherwise 128000. |
| `--no-stream` | Wait for each whole reply instead of showing it as it arrives. |
| `-y`, `--yes` | Run every tool without asking. |
| `--read-only` | No edits and no commands: read and report. |
| `--any-path` | Let file tools reach outside the project folder. |
| `--no-mcp` | Don't connect the MCP servers for this run. |
| `-C`, `--cwd DIR` | The project folder. |
| `--max-turns N` | Model calls allowed per request. Default 30. |
| `--json` | Print one JSON object: answer, outcome, turns, tokens, seconds. |
| `--log FILE` | Append every step to FILE as JSON lines. |
| `-q`, `--quiet` | Only the answer. |

On a terminal the answer is shown as it arrives. Progress lines go to stderr, so `rak "…" > answer.txt` holds only the answer.

Exit codes: `0` done, `1` failed, `2` bad usage, `3` a tool was not approved, `4` turn limit, `5` stuck repeating a call, `130` stopped with Ctrl-C.

## Safety

- **Edits and commands ask first.** `write_file`, `edit_file` and `run_shell` need a yes each time, unless you answer "always" or start with `-y`.
- **A no stops the request.** The model does not get another turn to try the same change a different way.
- **No terminal, no changes.** In a script or CI without `-y`, rak stops with exit code 3 instead of guessing.
- **File tools stay in the project folder** unless you pass `--any-path`. Symlinks are resolved before the check.
- **`run_shell` is not confined.** A command you approve can do anything your user can. Read it before you say yes.
- **An MCP server is a program you chose to run.** rak starts it with your user's rights, and trusts what it says about which of its tools are read-only.

## Tools

| Tool | What it does |
|---|---|
| `list_dir` | Lists a folder. |
| `search_files` | Finds files by name and by text inside them. Skips `.git`, `node_modules`, `target`, `.build`. |
| `read_file` | Reads a file. Files up to 40,000 characters come back in one call; small `limit`s are widened, because many small reads cost more than one large one. |
| `write_file` | Writes or appends a file. Refuses to overwrite a file that changed on disk since rak last read it. |
| `edit_file` | Replaces exact text. Tolerates trailing-whitespace differences; when the text is not found it shows the file's lines near the closest match. |
| `run_shell` | Runs a command with a time limit that kills the whole process tree. Background mode for servers. |

## MCP servers

Tools from outside rak come through the [Model Context Protocol](https://modelcontextprotocol.io). A server is a separate program, or a web address; its tools join rak's own.

```bash
# A program rak starts
rak mcp add files -- npx -y @modelcontextprotocol/server-filesystem .

# A server at a web address, with a key in a header
rak mcp add docs --url https://example.com/mcp -H 'Authorization: Bearer KEY'

rak mcp list
rak mcp remove files
```

| Command | What it does |
|---|---|
| `rak mcp add NAME -- COMMAND [ARG…]` | Adds a program. `-e KEY=VALUE` sets an environment variable for it. |
| `rak mcp add NAME --url URL` | Adds a web address. `-H 'Name: value'` sends a header. |
| `rak mcp list` | Lists the servers and tries each one: how many tools, or why it failed. |
| `rak mcp remove NAME` | Removes one. |

`add` saves the server, then connects once so a typo shows straight away. `--project` on `add` and `remove` uses `./.rak/mcp.json`, for this project only; without it the file is `~/.config/rak/mcp.json`, for all projects. Both are in the shape Claude Code and Cursor use, so an entry can be pasted across. A project's entry wins over one with the same name for all projects.

How the tools behave:

- **Names** are `server__tool`, so a server's `read_file` cannot be mistaken for rak's.
- **A tool the server marks read-only runs freely; any other asks first.** At the prompt, `s` allows all of that server's tools for the session.
- **`--read-only` keeps only the read-only tools.** `--no-mcp` leaves the servers out of a run.
- **Results over 20,000 characters are cut**, and Ctrl-C stops a call that is taking long.
- **Every server costs tokens on every request**, because its tool descriptions are sent each time. The reference filesystem server's 14 tools added 1,165 tokens to each model call when measured (2,187 against 1,022). Add the servers you use.

In a chat, `/mcp` lists the servers and their tools.

Not there yet: signing in to servers that need OAuth (a key in a header works), and MCP prompts and resources — only tools are used. The file holds keys in plain text; it is written readable by you only, and `rak mcp list` shows the names of variables and headers, not their values.

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

- saved sessions (`-c` to continue, `/save`, `/resume`)
- Ctrl-R to search the history, and the "still running" note during a long tool call
- web fetch and web search (an MCP server can supply them)
- native Anthropic and Gemini APIs (use an OpenAI-compatible endpoint)
- Naseem notes and skills
- a syntax check on written files

Known limits of the chat:

- Ctrl-C takes effect when the model next sends something. While a model is silent (still thinking), the first Ctrl-C waits; a second one quits rak.
- On Windows the chat falls back to plain line input: no menu, no arrow keys, no history.
- The window rak shows for a local Ollama model is the model's own maximum. Ollama may run it with a smaller one.

Not yet tested: Windows, and any public benchmark. The chat and the MCP client were driven by hand on macOS only; on Linux they build and their unit tests pass. The MCP client was tried with one real server (the reference filesystem server, a program); web-address servers are covered by a unit test against a stand-in. Auto mode (`-y`) has only been run inside the benchmark's containers.

## Development

```bash
cargo test
```

The code is in two layers. The library knows nothing about terminals; the `rak` command is one front end on top of it, and another (an editor plugin, a web server) could use the library the same way.

| Library (`src/`) | |
|---|---|
| `agent.rs` | The loop: ask the model, run the tools it calls, repeat. Retries, loop detection, context trimming. |
| `provider.rs` | The `Model` trait and the OpenAI-compatible client, streaming included. |
| `tools/` | The `Tool` trait and the six built-in tools; `Ctx` holds what they share. |
| `mcp/` | `config` (mcp.json), `client` (one server connection), and the bridge that turns a server's tools into `Tool`s. |
| `interrupt.rs`, `read_slice.rs`, `text.rs` | The stop flag, the file-read window, small text helpers. |

| Command (`src/cli/`) | |
|---|---|
| `args`, `session` | The command line; one run set up from it, and a request run and reported. |
| `chat` | The chat loop and its slash commands. |
| `approval`, `output`, `log` | Asking before changes; what is shown; `--log`. |
| `mcp_command` | `rak mcp add/list/remove`. |
| `editor`, `menu`, `markdown`, `welcome`, `style`, `term` | The interface's parts. Each is pure where it can be, and tested. |
| `settings`, `signals` | What is kept between runs; Ctrl-C. |

Dependencies only point down: `agent` uses `provider` and `tools`; `mcp` uses `tools`; nothing in the library uses `cli`. `rustfmt.toml` sets the formatting (`cargo fmt`).

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

| Run | rak | Passed | Time | Tokens in (median per run) | Tokens out |
|---|---|---|---|---|---|
| 2026-11-10 | 0.1.0-alpha.1, before the chat and MCP | 12/12 | 530 s | 182K (13.6K) | 44K |
| 2026-11-11 | after the chat, MCP and the module split | 11/12 | 552 s | 203K (14.1K) | 47K |

Both with kimi-k2.6:cloud through Ollama, two attempts per task. The system prompt and tools were the same in both runs.

The miss was task 1: the model fixed the three bugs the visible tests catch and stopped, leaving the fourth thing the docstrings ask for (`restock` raising `KeyError` for an unknown item), which only the hidden test checks. Four more attempts at task 1 passed 3 of 4. Across all eight attempts that task passed 6 times, so the first run's 2/2 was luck rather than the second run's 1/2 a regression.

These tasks are easy: a perfect score says the pipeline works, not how rak ranks. They are not the tasks sak was measured on, so the numbers do not compare with sak's.
