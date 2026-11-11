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

rak talks to any server that speaks OpenAI's chat-completions API (OpenAI, OpenRouter, Ollama, Hugging Face's router, vLLM, llama.cpp, LM Studio), and to Claude through Anthropic's own API.

```bash
# Ollama on this machine (the default address)
rak -m kimi-k2.6:cloud "explain what this project does"

# Claude: with ANTHROPIC_API_KEY set, a claude-* model uses Anthropic's API
export ANTHROPIC_API_KEY=...
rak -m claude-opus-5-5 "fix the failing test"

# Any other server
export RAK_BASE_URL=https://openrouter.ai/api/v1
export RAK_API_KEY=...
rak -m some/model "add a --verbose flag to the CLI"

# No task: a chat
rak -m kimi-k2.6:cloud
```

### Keys

```bash
rak keys
```

The first time, this creates `~/.config/rak/.env` from a template ([`.env.example`](.env.example)) that lists every key rak uses, with what each is for. Open it in any editor and fill in the ones you need; leave the rest empty. After that, `rak keys` shows which keys are set and where each comes from, never the values.

rak reads the environment first, then `./.env` in the project, then `~/.config/rak/.env`, the first one found winning. Values read from the files stay inside rak: commands the agent runs with `run_shell` don't see them. The file is created readable by you only.

| Key | For |
|---|---|
| `ANTHROPIC_API_KEY` | Claude through Anthropic's API |
| `RAK_API_KEY`, `OPENAI_API_KEY` | a chat-completions server that needs a key |
| `TAVILY_API_KEY` | `web_search` |
| `RAK_MODEL`, `RAK_PROVIDER`, `RAK_BASE_URL` | defaults for `-m`, `--provider` and `--base-url` (optional) |

The chat's welcome panel lists the `.env` files it read.

### Claude and prompt caching

An agent sends the whole conversation again with every model call, so most of what it pays for is input it sent before. Prompt caching lets the server read that part back for a fraction of the price: on Anthropic's API a cache read costs a tenth of normal input, or less on the newest models.

rak speaks Anthropic's Messages API directly for this, rather than going through its OpenAI-compatible endpoint:

- **Caching** uses two markers: one on the system prompt (which covers the tools too, since they come first), and the automatic one that follows the end of the conversation. Each call then reads back everything the previous call sent.
- **Thinking blocks** are sent back unchanged with each request, as the API requires.
- **Refusals.** For the models that can decline a request (Claude Fable 5.2 and 5.1, Opus 5.5 and 5, Sonnet 5.5), rak asks for Anthropic's server-side fallback: on a decline, the API answers with the model it recommends in the same call. A decline with no fallback ends the request and says why.
- **The context window** is read from Anthropic's models list.

After each request rak shows how much of the input came from the cache (`6220 tokens in (3000 cached)`), and `/usage` and `--json` include it. Servers on the chat-completions API that report cached tokens show them the same way; Ollama's cloud models do.

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
| `--provider NAME` | `openai` (the chat-completions API) or `anthropic`. Or set `RAK_PROVIDER`. Default: `anthropic` for a `claude-*` model when `ANTHROPIC_API_KEY` is set and no `--base-url` is given, or when the address is Anthropic's; otherwise `openai`. |
| `--base-url URL` | Server address. Or set `RAK_BASE_URL`. Default `http://localhost:11434/v1`, or `https://api.anthropic.com` for `anthropic`. |
| `--api-key-env NAME` | Environment variable holding the key. Default `RAK_API_KEY`, then `OPENAI_API_KEY`; `ANTHROPIC_API_KEY` first for `anthropic`. |
| `--effort LEVEL` | Reasoning effort: `low`, `medium`, `high`, `xhigh` or `max`. |
| `--max-tokens N` | Longest reply to ask for. |
| `--context N` | The model's context window in tokens. Default: what Anthropic or an Ollama server reports for the model, otherwise 128000. |
| `--no-stream` | Wait for each whole reply instead of showing it as it arrives. |
| `-y`, `--yes` | Run every tool without asking. |
| `--read-only` | No edits and no commands: read and report. |
| `--any-path` | Let file tools reach outside the project folder. |
| `--no-mcp` | Don't connect the MCP servers for this run. |
| `-C`, `--cwd DIR` | The project folder. |
| `--max-turns N` | Model calls allowed per request. Default 30. |
| `--json` | Print one JSON object: answer, outcome, turns, tokens (cached ones too), seconds. |
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
| `write_file` | Writes or appends a file. Refuses to overwrite a file that changed on disk since rak last read it. Checked before writing (below). |
| `edit_file` | Replaces exact text. Tolerates trailing-whitespace differences; when the text is not found it shows the file's lines near the closest match. Checked before writing (below). |
| `run_shell` | Runs a command with a time limit that kills the whole process tree. Background mode for servers. |
| `web_search` | Searches the web through [Tavily](https://tavily.com): the most relevant pages with a short extract of each, and a direct answer when there is one. Only there when `TAVILY_API_KEY` is set; it only reads, so it needs no approval and stays in read-only mode. Each search spends a Tavily credit (`deep` spends two). |

**The write check** (from SwiftAgentKit's `WriteVerifier`). Before `write_file` or `edit_file` changes a file, the file as it would be afterwards is checked; if the check fails, the file is left as it was and the model is told why.

- **Damaged content:** many lines starting with a diff's `+` in a code file means patch text leaked into the file. The model is told to resend.
- **Syntax:** JSON is parsed by rak itself; files that by convention allow comments (`tsconfig.json`, `.vscode/*.json`, `.jsonc`) may have them. Python, JavaScript, shell scripts, Swift and Dart go to their own parser (`python3`, `node --check`, `bash -n`, `swiftc -parse`, `dart format`). The model is told resending the same content will fail the same way, and which interpreter judged it.
- **When a machine has several copies of an interpreter, the newest judges,** so an old system Python does not reject code written for the one in use.
- **A check that cannot run passes.** No parser installed, a file over 2 MB, a parser that hangs past 10 seconds or crashes: the write goes ahead. A false rejection would block the agent, which is worse than a miss.

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
- fetching a whole web page (an MCP server can supply it)
- Gemini's own API (use an OpenAI-compatible endpoint)
- Naseem notes and skills

Known limits of the chat:

- Ctrl-C takes effect when the model next sends something. While a model is silent (still thinking), the first Ctrl-C waits; a second one quits rak.
- On Windows the chat falls back to plain line input: no menu, no arrow keys, no history.
- The window rak shows for a local Ollama model is the model's own maximum. Ollama may run it with a smaller one.
- With Anthropic's API, trimming old tool output near a full context window changes earlier messages, which ends the cache from that point; the next call writes it again.

Not yet tested: Windows, and any public benchmark. Anthropic's real API was tried on Claude Haiku 5.5 (2026-11-11): the first call wrote 1,514 tokens to the cache and the second read all 1,514 back, writing only the 87 new ones. Its other paths (thinking blocks sent back, refusal fallbacks) are tested against a local stand-in. The chat and the MCP client were driven by hand on macOS only; on Linux they build and their unit tests pass. The MCP client was tried with one real server (the reference filesystem server, a program); web-address servers are covered by a unit test against a stand-in. Auto mode (`-y`) has only been run inside the benchmark's containers.

## Development

```bash
cargo test
```

The code is in two layers. The library knows nothing about terminals; the `rak` command is one front end on top of it, and another (an editor plugin, a web server) could use the library the same way.

| Library (`src/`) | |
|---|---|
| `agent.rs` | The loop: ask the model, run the tools it calls, repeat. Retries, loop detection, context trimming. |
| `provider.rs` | The `Model` trait and the OpenAI-compatible client, streaming included. |
| `anthropic.rs` | Claude through Anthropic's Messages API: caching, thinking blocks, refusal fallbacks. |
| `tools/` | The `Tool` trait and the six built-in tools; `Ctx` holds what they share; `verify` is the write check. |
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
| 2026-11-11 | with the write check | 12/12 | 434 s | 258K (19.3K), 222K of it cached | 41K |

All with kimi-k2.6:cloud through Ollama, two attempts per task. The system prompt and tool descriptions were the same in every run.

The third run is the first that records cached tokens: 86% of the input was read from Ollama's cache. Ollama's cloud did that caching on its own; the earlier runs most likely had it too, unrecorded. The write check rejected no write in this run, so it caused no false alarms, but these tasks gave it nothing to catch either. Input was higher than in the earlier runs because the model took more turns; with this few attempts per task, that is within the run-to-run spread.

The miss was task 1: the model fixed the three bugs the visible tests catch and stopped, leaving the fourth thing the docstrings ask for (`restock` raising `KeyError` for an unknown item), which only the hidden test checks. Four more attempts at task 1 passed 3 of 4. Across all eight attempts that task passed 6 times, so the first run's 2/2 was luck rather than the second run's 1/2 a regression.

### Head to head with sak

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

### Head to head with Hermes

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
