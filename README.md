# rak — Rust Agent Kit

A small coding agent for the terminal. One binary of a few megabytes, no runtime to install, for Linux and macOS.

rak is the cross-platform sibling of [sak](https://github.com/ayman3000/sak). sak is built on SwiftAgentKit and stays the Mac tool; rak carries the same prompt, the same tool designs and the same loop to everything else.

> **Status: early alpha (0.1.0-alpha.9).** The core works and is tested. See [What is missing](#what-is-missing).

**Contents:** [What it does](#what-it-does) · [Install](#install) · [Getting started](#getting-started) · [The chat](#the-chat) · [Providers and models](#providers-and-models) · [Tools](#tools) · [Safety](#safety) · [Project notes](#project-notes-the-rak-folder) · [Large projects](#large-projects) · [Skills](#skills) · [MCP servers](#mcp-servers) · [Command line](#command-line) · [As a library](#as-a-library) · [What is missing](#what-is-missing) · [Development](#development) · [Benchmarks](#benchmarks)

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

What sets it apart:

- **Small and light.** One file that starts at once; a short request took about 4 MB of memory in the alpha.1 benchmark ([measured](docs/benchmarks.md#head-to-head-with-hermes)).
- **Cheap per call.** A short prompt and a lean tool set, so every model call sends less.
- **Any model.** Fifteen providers by name, any OpenAI-compatible server by address, Claude through Anthropic's own API with prompt caching, and fallback to another provider when one fails.
- **Asks before it changes anything**, and shows you exactly what it will run.
- **Remembers the project** in a plain `rak/` folder you can read and commit.

## Install

On Linux or macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/ayman3000/rak/main/install.sh | sh
```

The script downloads the newest release for your system, checks its SHA-256 checksum, and puts `rak` in `~/.local/bin`. Nothing else is changed.

- `RAK_VERSION=v0.1.0-alpha.9` picks a release (default: the newest, pre-releases included); `RAK_INSTALL_DIR` picks the folder.
- **Builds:** Linux on x86-64 and ARM (static, any distribution), macOS on Apple Silicon and Intel. Windows is not supported yet.
- **macOS:** the builds are not notarized. Installed with the script they just run; a build downloaded in a web browser needs `xattr -d com.apple.quarantine rak` first.

**From source**, with Rust 1.88 or newer ([rustup.rs](https://rustup.rs)). This builds `main`, which may be ahead of the newest release:

```bash
cargo install --git https://github.com/ayman3000/rak --locked
```

**To update**, run the install command again. If you used both at some point, remove the other copy (`cargo uninstall rak`) so the right one runs.

## Getting started

**1. Create your key file.**

```bash
rak keys
```

This creates `~/.config/rak/.env` (readable by you only) and lists every provider rak knows, with whether its key is set.

**2. Fill it in.** Open it in any editor (`open -t ~/.config/rak/.env` on macOS, `nano ~/.config/rak/.env` on Linux). Put each key right after its `=` (no quotes), and say which provider and model to use:

```bash
# 1. API keys: one per provider. Fill in the ones you have.
OPENAI_API_KEY=
ANTHROPIC_API_KEY=sk-ant-…
OLLAMA_API_KEY=
…

# 2. Selected provider and model
RAK_PROVIDER=anthropic
RAK_MODEL=claude-sonnet-5-5
```

Leave `RAK_MODEL` empty and rak lists the provider's models to pick from. With Ollama on your machine you need no key at all: leave `RAK_PROVIDER` empty and name a model, such as `kimi-k2.6:cloud`.

**3. Check, then start.** Run `rak keys` again: each key you filled in should say `set`, and your provider should be marked `▸`. Then:

```bash
rak                                    # a chat
rak "explain what this project does"   # one request
```

**4. In a project, type `/init`** once. rak looks at the project and writes its notes in `rak/`, so every later session starts already knowing it (see [Project notes](#project-notes-the-rak-folder)).

<details>
<summary><b>Upgrading from an older key file</b></summary>

Your key file is never overwritten. If it was made from an older template, `rak keys` says so; then `rak keys --update` rewrites it from the current template with each of your values on its line, keeps any value the template has no line for in a section at the end, and saves your old file as `~/.config/rak/.env.bak`. It names what moved, never a value, and running it again changes nothing.

What changed meaning with named providers (`--update` warns about the second and third):

- **Each provider gets only its own key.** `OPENAI_API_KEY` goes only to OpenAI, not to any server you name.
- **`RAK_PROVIDER=openai`** means OpenAI itself. If you used it with `RAK_BASE_URL` for another server, set `RAK_PROVIDER` to that server's name and empty `RAK_BASE_URL`.
- **`RAK_API_KEY`** goes only to a server you name with `--base-url`.
- **The last model you picked** is remembered for each provider. An earlier one carries over as Ollama's.

</details>

## The chat

Started without a task, rak opens a chat:

```
╭─ rak 0.1.0-alpha.9 ──────────────────────────────────────────────────────────╮
│ ██████   █████  ██   ██            │ Tools                                   │
│ ██   ██ ██   ██ ██  ██             │ files   list_dir, search_files,         │
│ ██████  ███████ █████              │         code_map, read_file, +4 more    │
│ ██   ██ ██   ██ ██  ██             │ shell   run_shell                       │
│ ██   ██ ██   ██ ██   ██            │ plan    todo, delegate                  │
│ a coding agent for your terminal   │ web     fetch_url                       │
│                                    │ skills  use_skill, save_skill           │
│ model   kimi-k2.6:cloud            │ more    remember, save_routine          │
│ via     ollama.com                 │                                         │
│ folder  ~/projects/shop            │                                         │
│ branch  main                       │                                         │
│ mode    asks before changes        │                                         │
│ window  262K tokens                │                                         │
│ config  ~/.config/rak/.env         │                                         │
│ skills  2 (/skills)                │                                         │
│                                                                              │
│ 16 tools · /help for commands · Ctrl-C stops a request · Ctrl-D quits        │
╰──────────────────────────────────────────────────────────────────────────────╯

── kimi-k2.6:cloud · ctx 0/262K ▱▱▱▱▱▱▱▱▱▱ 0% · shop (main) · asks before changes
❯ /mo
❯ /model [name]     show or switch the model (no name: pick from a list)
  /models           list the models the provider offers
```

- **The rule above the prompt** shows the model, how full its context is, the folder and branch, and the mode.
- **Type `/`** for the commands. The list narrows as you type; ↑ and ↓ choose, Tab completes, Enter runs, Esc closes it.
- **Answers appear as they are written**, with Markdown shown as headings, bullets and code.
- **Ctrl-C stops a request** (the answer being written, or a running command) and keeps the chat. A second Ctrl-C quits.
- **History** is kept between runs (↑ and ↓). Pasted text keeps its line breaks; a line ending in `\` continues on the next.
- **After each request** a line shows turns, tool calls (a helper's counted apart: `3 tool calls (+14 in helpers)`), tokens in and out, how many came from the cache, and the time.

| Command | What it does |
|---|---|
| `/model [name]` | Show or switch the model. Without a name: pick from a list (↑/↓, Enter, Esc cancels, type to narrow; a name the list doesn't have can be typed and used). |
| `/models` | List the models the provider offers. |
| `/provider [name]` | Show or switch the provider: its own address and key, and the model you last picked there. The conversation comes along. Without a name: a list that dims providers whose key isn't set. |
| `/init` | Set up this project's notes in `rak/`, or bring them up to date. |
| `/notes` | Show the project's rules and notes (project, roadmap without Done, the latest journal lines). |
| `/tools`, `/mcp`, `/skills` | List the tools, the MCP servers and their tools, the skills. `/skills add URL` adds a skill (it shows it and asks first). |
| `/resume [N]` | This folder's saved chats, newest first; `/resume N` picks one up. |
| `/yes` | Turn auto-approve on or off. |
| `/usage` | Tokens, tool calls, the context used and the window, fallbacks. |
| `/clear` | Start a fresh conversation. The last one stays saved. |
| `/help`, `/exit` | The list of commands; quit (or Ctrl-D). |

## Providers and models

rak knows these providers by name. Each gets its own address and only its own key:

| `RAK_PROVIDER` / `--provider` | Who | Key | Tried for real |
|---|---|---|---|
| `ollama` (the default) | Ollama on this machine, its `:cloud` models included | none | ✅ 2026-11-12 |
| `ollama-cloud` | Ollama's cloud at ollama.com, without Ollama installed | `OLLAMA_API_KEY` | ✅ 2026-11-12 |
| `anthropic` | Claude through Anthropic's API, with prompt caching | `ANTHROPIC_API_KEY` | ✅ 2026-11-12 |
| `openai` | OpenAI | `OPENAI_API_KEY` | not yet |
| `gemini` | Google Gemini | `GEMINI_API_KEY` | not yet |
| `zai`, `zai-coding` | Z.ai (GLM); `zai-coding` for GLM Coding Plan keys | `ZAI_API_KEY` | not yet |
| `openrouter` | OpenRouter | `OPENROUTER_API_KEY` | not yet |
| `huggingface` | Hugging Face's router | `HF_TOKEN` | not yet |
| `groq` | Groq | `GROQ_API_KEY` | not yet |
| `deepseek` | DeepSeek | `DEEPSEEK_API_KEY` | not yet |
| `mistral` | Mistral | `MISTRAL_API_KEY` | not yet |
| `lmstudio`, `llamacpp`, `vllm` | those servers on this machine | none | not yet |

"Not yet" means set up from the provider's documentation but no real request made. Tried one? Tell us how it went, in an issue. The list is data, [`src/providers.json`](src/providers.json): adding a provider is one line there.

### Choosing a provider and model

| Where | How | Lasts |
|---|---|---|
| The key file | `RAK_PROVIDER=…` and `RAK_MODEL=…` | until you change it |
| The command line | `rak --provider groq -m MODEL "…"` | that run |
| In a chat | `/provider NAME`, `/model NAME` | that chat; the model is remembered |

**Which model is used:** in a chat, `-m` if given, then the model you last picked for that provider, then `RAK_MODEL`, then a list to pick from. A one-off run (`rak "…"`) uses `-m`, then `RAK_MODEL`, then your last pick, so scripts stay predictable. `RAK_MODEL` belongs to the provider in `RAK_PROVIDER`, not to one chosen with `--provider`.

```bash
# Ollama on this machine: nothing to set
rak -m kimi-k2.6:cloud "explain what this project does"

# Another provider, for one run
rak --provider openrouter -m some/model "add a --verbose flag to the CLI"

# With ANTHROPIC_API_KEY set, a claude-* model goes to Anthropic on its own
rak -m claude-haiku-5-5 "summarize the README"

# A server rak doesn't know by name: its address, and RAK_API_KEY if it needs one
rak --base-url https://example.com/v1 -m some-model "…"
```

### Keys and settings

rak reads the environment first, then `./.env` in the project, then `~/.config/rak/.env`; the first one with a value wins. Within one file, the last line set wins, and an empty line erases nothing. `rak keys` and the chat's welcome panel show what is set and where it came from, never a key itself, and say what was set more than once.

| Setting | For |
|---|---|
| a provider's key (table above) | that provider, and only that provider |
| `TAVILY_API_KEY` | `web_search` |
| `RAK_PROVIDER`, `RAK_MODEL` | the selected provider and model |
| `RAK_BASE_URL` | the selected provider at another address, e.g. Ollama on another machine |
| `RAK_API_KEY` | the key for a server named only by `--base-url` |

**A key never reaches another provider's server.** Defaults from the file never mix with flags: `--provider` uses that provider's own address even when `RAK_BASE_URL` is set, and `--base-url` drops the selected provider and its key. Values read from the files stay inside rak: commands the agent runs don't see them.

### When a provider fails

rak carries on with another provider instead of stopping, with nothing to set up:

1. **The same model is asked again first**, twice, when that can help (network trouble, a busy or rate-limited server). Out of credit, a bad key or an unknown model move on at once.
2. **Then the next provider:** each other provider whose key is set, in the order the key lines appear in your key file, then the servers on this machine, each with the model you last picked for it. A provider you have never used in rak is skipped, so a key sitting in the file is never spent on its own.
3. **In a chat, rak asks first:** `ollama-cloud · kimi-k2.6 failed: … Continue with anthropic · claude-sonnet-5-5? [y/n]`. With `-y`, or in a script, it switches and says so.
4. **The step is done again** on the new model with the conversation so far. Your next message goes back to the provider you chose, unless that one can't work at all (a bad key, a missing model).

**A silent server counts as a failure.** A streamed answer that sends nothing for too long is stopped and retried: 5 minutes for Anthropic, 10 for other cloud servers, 15 for a server on this machine, a minute or two more for a very long prompt (from SwiftAgentKit's `StreamStallPolicy`).

Every switch is shown (`↪ now using …`), logged with `--log`, listed in `--json` (`fell_back_to`) and counted in `/usage`. A refusal, a stop, or a request rak itself built wrongly does not fall back: another model would not do better.

### Claude and prompt caching

An agent sends the whole conversation again with every call, so most of what it pays for is input it sent before. Prompt caching lets the server read that part back for a fraction of the price: on Anthropic's API a cache read costs a tenth of normal input, or less.

With `--provider anthropic` (or a `claude-*` model and `ANTHROPIC_API_KEY` set), rak speaks Anthropic's Messages API directly:

- **Caching** marks the system prompt (which covers the tools) and the end of the conversation, so each call reads back everything the previous one sent.
- **Thinking blocks** are sent back unchanged, as the API requires.
- **Refusals:** for the models that can decline a request, rak asks for Anthropic's server-side fallback, so a decline is answered by the recommended model in the same call.
- **The context window** is read from Anthropic's models list.

After each request rak shows how much input came from the cache (`6220 tokens in (3000 cached)`). Other servers that report cached tokens (Ollama's cloud models do) show them the same way.

## Tools

| Tool | What it does | Asks first |
|---|---|---|
| `list_dir` | Lists a folder. | |
| `search_files` | Finds files by name and by text inside them. Skips `.git`, `node_modules`, `target`, `.build`. | |
| `code_map` | Maps the project: every source file with the functions, types and classes it defines, and their lines, for 11 languages. Source folders first, tests and docs last; past about 14,000 characters it names the folders it didn't map. | |
| `read_file` | Reads a file, up to 40,000 characters in one call, or just `start_line`–`end_line`. | |
| `view_image` | Shows the model an image file (a screenshot, say). Offered only when the model can see. | |
| `write_file` | Writes or appends a file. Refuses to overwrite a file that changed on disk since rak read it. | yes |
| `edit_file` | Replaces exact text, tolerating trailing-whitespace differences; when the text isn't found it shows the closest lines. | yes |
| `apply_patch` | Applies a unified diff to one file: several changes in one approved step, all or nothing. Hunks are placed by their lines, not their `@@` numbers. | yes |
| `run_shell` | Runs a command with a time limit that kills the whole process tree. Background mode for servers. | yes |
| `todo` | The agent's task list for work with three or more steps, shown to you as it changes. If the agent says it is done with steps still open, rak sends it back once. | |
| `delegate` | Hands a question to a helper agent with a fresh context. The helper has only `list_dir`, `search_files`, `code_map`, `read_file`, `fetch_url` (and `web_search` with a key): no shell, no edits. Only its answer comes back, so a big search doesn't fill the main context. Several `delegate` calls in one reply run at the same time: up to 3 on a commercial cloud API, 2 on Ollama's cloud, 1 on a model on this machine (the rest wait their turn). Their steps show indented, tagged `[1]`, `[2]`… | |
| `web_search` | Searches the web through [Tavily](https://tavily.com). Only there when `TAVILY_API_KEY` is set; each search spends a Tavily credit. | |
| `fetch_url` | Reads one public web page as Markdown, text or HTML. Addresses on this machine or a private network are refused, even through a redirect. | |
| `remember` | Keeps a one-line note for later sessions, for this project or for all. | yes |
| `save_routine`, `run_routine` | Saves steps that just worked, with `{placeholders}`, so they run again back to back with no model calls between them. Each step still asks like the model's own call. Same format as sak and Naseem. | yes |

Two more appear only when needed: `load_tools` for a [large MCP server](#mcp-servers), and `artifact_read` / `artifact_search` once [old output has been set aside](#large-projects).

**Every write is checked first** (from SwiftAgentKit's `WriteVerifier`). Before `write_file`, `edit_file` or `apply_patch` changes a file, the new content is checked, and if the check fails the file is left as it was and the model is told why:

- **Leaked patch text:** many lines starting with a diff's `+` in a code file.
- **Syntax:** JSON is parsed by rak itself (comments allowed where the convention allows them). Python, JavaScript, shell scripts, Swift and Dart go to their own parser; when several copies are installed, the newest judges.
- **A check that cannot run passes** (no parser, a file over 2 MB, a parser that hangs past 10 seconds), because a false rejection would block the agent.

## Safety

- **Edits and commands ask first**, every time, unless you answer "always" or start with `-y`. The prompt shows the whole command, every line of it.
- **A no stops the request.** The model doesn't get another turn to try the same change a different way.
- **No terminal, no changes.** In a script or CI without `-y`, rak stops with exit code 3 instead of guessing.
- **A tool error is not passed off as done.** When the step before the answer had a tool error (an edit that didn't apply), rak asks the model once to fix it or say plainly what failed; at most three times a request.
- **A request that edited files must run the project's gate**, if it has one (see [Project notes](#project-notes-the-rak-folder)).
- **File tools stay in the project folder** unless you pass `--any-path`. Symlinks are resolved first.
- **`run_shell` is not confined.** A command you approve can do anything your user can. Read it before you say yes.
- **Starting in your home folder or `/`** shows a warning first, since the agent could then reach everything below it.
- **An MCP server is a program you chose to run.** rak starts it with your rights and trusts what it says about which tools are read-only.

## Project notes: the `rak/` folder

rak keeps a project's notes in a visible `rak/` folder inside it: plain Markdown to read, edit and commit.

| File | What it holds |
|---|---|
| `rak/project.md` | What the project is, who it is for, the stack, how to build and test. |
| `rak/roadmap.md` | `## Done`, `## In progress`, `## Next`, `## Known issues`. |
| `rak/journal.md` | One dated line per request that changed something: `- 2026-11-13 20:51 [fix] — asked → what changed`. Past 30 lines the oldest move to `journal-archive.md`. |
| `rak/rules.md` | Your instructions for this project (read along with `AGENTS.md` and `CLAUDE.md`), with an optional `## Gate`. |
| `rak/memory.md` | Notes kept with `remember`. |

How it works:

- **Start with `/init`.** The agent looks at the project (map, README, build and test files, git log) and writes `project.md` and `roadmap.md`, each write approved. Run it again later to bring them up to date. rak never creates the folder on its own.
- **Loaded, not read.** Every session starts with the rules, `project.md` and the roadmap's open sections in its instructions, so it starts oriented instead of spending its first steps reading.
- **Kept current.** After a request that changed a file or ran a command, one small model call writes the journal line and moves or adds roadmap cards. rak applies those edits itself: a card it can't find is skipped and said so, and nothing is ever deleted. A plain question costs nothing extra.
- **The gate.** A `## Gate` section in `rak/rules.md` (or `AGENTS.md`) with a fenced command:

  ````markdown
  ## Gate
  ```
  cargo test && cargo clippy -- -D warnings
  ```
  ````

  A request that edited files isn't done until that command has run after the last edit. If the agent says "done" without running it, rak sends it back (twice at most).

`--no-notes` turns the after-request update off; read-only mode never writes. Notes kept with `remember` for all projects live in `~/.config/rak/memory.md`; `--no-memory` leaves both memory files out.

## Large projects

What helps the agent on a project too big to read whole:

- **A map first.** `code_map` shows where everything is, and `read_file` with line numbers reads only what's needed.
- **A plan it keeps.** `todo` holds the steps, and is never dropped when the context fills.
- **Helpers for searching.** `delegate` reads many files in its own context and returns only the answer; independent questions run side by side.
- **Chats are saved** after every request (30 per project folder). `rak -c` continues the last one; `/resume` picks any.
- **When the context fills,** old tool outputs are replaced by a short note naming the call (`read_file src/main.rs`). The output itself is set aside (`out-3`), and the agent reads it back with `artifact_read` or searches it with `artifact_search` instead of running the tool again.
- **When that isn't enough,** the older part of the conversation is replaced by a summary the model writes; the newest turns and the task list stay word for word. If the server still says the request is too long, rak summarizes once more and asks again.

## Skills

A skill is know-how in Markdown: how to build an iOS app from the terminal, how this team deploys, how a library is meant to be used. The prompt lists each skill as one line, and the agent loads the full instructions with `use_skill` when a task matches.

- **Where:** `.rak/skills/` in the project, then `~/.config/rak/skills/`; a project's skill wins over one with the same name.
- **Two formats:** `NAME.md` as in SwiftAgentKit and sak (`# Name`, `Description:`, then the instructions), or a folder with a `SKILL.md` as in Claude's Agent Skills. Files a skill ships go beside it.
- **Built in:** `iOS app` (macOS only: builds an app with `swiftc` and runs it in the Simulator) and `Write a skill`. A skill of yours with the same name replaces a built-in one; `--no-skills` leaves all out.
- **The agent writes skills too.** Ask for one and it interviews you, shows the draft and saves it with `save_skill` once you agree. After you correct it, it may keep the lesson as a skill. Every save asks first and shows the whole skill.

**Adding skills from elsewhere:**

```bash
rak skills add https://github.com/anthropics/skills/tree/main/skills/pdf
rak skills add ./deploy.md --project
rak skills                    # list them
rak skills remove pdf
```

`rak skills add` takes a web address (a GitHub file or skill folder, with the files it ships) or a file. Before saving it shows the skill and points out lines worth reading first: commands piped into a shell, deletions, `~/.ssh`, `.env`, text telling the agent to ignore you. That list is a hint, not a verdict: read the skill.

## MCP servers

Tools from outside rak come through the [Model Context Protocol](https://modelcontextprotocol.io). A server is a separate program or a web address; its tools join rak's own.

```bash
rak mcp add files -- npx -y @modelcontextprotocol/server-filesystem .        # a program
rak mcp add docs --url https://example.com/mcp -H 'Authorization: Bearer KEY'  # a web address
rak mcp list                                                                  # try each one
rak mcp remove files
```

- **Where:** `~/.config/rak/mcp.json` for all projects, or `./.rak/mcp.json` with `--project`. Same shape as Claude Code and Cursor, so entries can be pasted across. `add` connects once so a typo shows straight away; `-e KEY=VALUE` sets an environment variable for a program.
- **Names** are `server__tool`, so a server's `read_file` can't be mistaken for rak's.
- **Approval:** a tool the server marks read-only runs freely; any other asks first (`s` at the prompt allows all of that server's tools for the session). `--read-only` keeps only read-only tools; `--no-mcp` leaves servers out.
- **Large servers load when needed.** Tool descriptions are sent on every call, so a server whose descriptions pass about 1,000 tokens is named in one line instead, and its tools are sent only after the model calls `load_tools` for it.
- **Not there yet:** OAuth sign-in (a key in a header works), and MCP prompts and resources.

## Command line

| Option | What it does |
|---|---|
| `-m`, `--model NAME` | Model to use. |
| `--provider NAME` | Who runs the model (see [Providers](#providers-and-models)). |
| `--base-url URL` | Another address: a server rak doesn't know by name, or a provider somewhere else. |
| `--api-key-env NAME` | The variable holding the key to send. |
| `--effort LEVEL` | Reasoning effort: `low`, `medium`, `high`, `xhigh` or `max`. |
| `--max-tokens N` | Longest reply to ask for. |
| `--context N` | The model's context window. Default: what Anthropic or Ollama reports, otherwise 128000. |
| `--no-stream` | Wait for each whole reply instead of showing it as it arrives. |
| `-y`, `--yes` | Run every tool without asking. Use it only inside a container. |
| `--read-only` | No edits and no commands: read and report. |
| `--any-path` | Let file tools reach outside the project folder. |
| `--no-mcp`, `--no-skills`, `--no-memory`, `--no-notes` | Leave out MCP servers, skills, memory notes, or the after-request notes update. |
| `-c`, `--continue` | Continue this folder's last chat (`rak -c "and now the tests"`). |
| `-C`, `--cwd DIR` | The project folder. |
| `--max-turns N` | Model calls allowed per request. Default 30. |
| `--json` | Print one JSON object: answer, outcome, turns, tool calls, tokens, seconds. |
| `--log FILE` | Append every step to FILE as JSON lines. |
| `--record FILE`, `--replay FILE` | Record the model's replies, then play them back offline to rerun a session while fixing rak (a developer tool; SwiftAgentKit's scenario format). |
| `-q`, `--quiet` | Only the answer. |

Other commands: `rak keys`, `rak skills …`, `rak mcp …`.

Progress goes to stderr, so `rak "…" > answer.txt` holds only the answer. Exit codes: `0` done, `1` failed, `2` bad usage, `3` a tool was not approved, `4` turn limit, `5` stuck repeating a call, `130` stopped with Ctrl-C.

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

A runnable version is in [examples/minimal.rs](examples/minimal.rs). Add a tool by implementing `rak::tools::Tool`; plug in another model API by implementing `rak::provider::Model`.

## What is missing

Compared with sak, rak doesn't have yet:

- Ctrl-R to search the history, and a "still running" note during a long tool call
- Gemini's own API (`--provider gemini` uses its OpenAI-compatible endpoint)
- Markdown tables drawn as tables in the terminal (they show as raw pipes)
- Windows

Known limits:

- Ctrl-C takes effect when the model next sends something; while it is silent (thinking), a second Ctrl-C quits rak.
- The window shown for a local Ollama model is the model's maximum; Ollama may run it with a smaller one.
- On Anthropic's API, trimming or summarizing near a full window ends the cache from that point; the next call writes it again.
- Only Ollama, Ollama's cloud and Anthropic have been tried for real. The chat was driven by hand on macOS only (on Linux it builds and its tests pass). `-y` has only run inside containers. More in [docs/field-notes.md](docs/field-notes.md).

## Development

```bash
cargo test
cargo build --release
```

The code is in two layers: a library that knows nothing about terminals, and the `rak` command on top of it. Dependencies only point down; nothing in the library uses `cli`.

| Library (`src/`) | |
|---|---|
| `agent.rs` | The loop: ask the model, run the tools it calls, repeat. Retries, loop detection, the repair nudge, the finish and gate checks, trimming. |
| `compact.rs`, `artifacts.rs` | Summarizing a full context; output set aside when trimmed. |
| `provider.rs`, `anthropic.rs` | The `Model` trait and the OpenAI-compatible client; Claude's Messages API. |
| `providers.rs`, `providers.json`, `fallback.rs`, `stall.rs` | The named providers; moving to another when one fails; the silent-server limits. |
| `tools/` | The `Tool` trait and the built-in tools (`fs`, `patch`, `shell`, `map`, `todo`, `image`, `web`, `fetch`); `verify` is the write check. |
| `delegate.rs` | The helper agent run inside a tool call. |
| `notes.rs`, `memory.rs`, `routines.rs`, `sessions.rs` | The `rak/` notes and gate; `remember`; routines; saved chats. |
| `mcp/`, `skills/` | MCP config, client and bridge; reading, saving and fetching skills. |
| `replay.rs` | `--record` and `--replay`. |
| `interrupt.rs`, `read_slice.rs`, `text.rs` | The stop flag, the file-read window, text helpers. |

| Command (`src/cli/`) | |
|---|---|
| `args`, `session`, `chain` | The command line; one run set up and reported; the fallback chain's questions. |
| `chat`, `commands` | The chat loop and its slash commands. |
| `approval`, `output`, `log` | Asking before changes; what is shown; `--log`. |
| `mcp_command`, `skills_command`, `keys_command` | `rak mcp`, `rak skills`, `rak keys`. |
| `editor`, `menu`, `picker`, `markdown`, `welcome`, `style`, `term` | The interface's parts, each pure where it can be, and tested. |
| `settings`, `signals` | What is kept between runs; Ctrl-C. |

**Checks and releases.** Every push to `main` and every pull request runs the tests on Linux and macOS, the format and lint checks, and a build with the oldest supported Rust (`.github/workflows/ci.yml`). A version tag matching `Cargo.toml` builds the four release archives and publishes them (`.github/workflows/release.yml`); a tag with a `-` makes a pre-release:

```bash
git tag v0.1.0-alpha.9 && git push origin v0.1.0-alpha.9
```

MIT licence.

## Benchmarks

Six small Python tasks, each in a fresh container, graded by tests the agent never sees. With kimi-k2.6:cloud on 2026-11-11:

| | Passed | Tokens in | Memory (one-word reply) |
|---|---|---|---|
| rak 0.1.0-alpha.1 vs sak 0.3.0-alpha | 17/18 vs 18/18 | 385K vs 511K | |
| rak 0.1.0-alpha.1 vs Hermes Agent 0.22.4 | 16/18 vs 16/18 | 314K vs 1,660K | 4.0 MB vs 357 MB |

The tasks are easy: a perfect score says the pipeline works, not how rak ranks. The setup, every run and what the numbers do and don't say are in [docs/benchmarks.md](docs/benchmarks.md).
