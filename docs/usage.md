# Using rak

Setting up, the chat, and every command-line option. Back to the [README](../README.md).

**Contents:** [Getting started](#getting-started) · [The chat](#the-chat) · [Command line](#command-line)

## Getting started

**1. Run `rak`.** The first time, it sets itself up:

- **Pick a provider** from the list. Ollama on your machine comes first when it is running, and needs no key.
- **Paste its key.** It is not shown as you type or paste, and it is saved to `~/.config/rak/.env`, readable by you only. A key the provider refuses is caught here, not at your first message (for OpenRouter and Hugging Face, whose model lists need no key, rak checks the key against the account first; that runs no model and costs nothing).
- **Pick a model** from the provider's own list (type to narrow it).

Then the chat starts. Run `rak setup` any time to switch provider or model, or to give a new key; the rest of the file stays as it was.

```bash
rak                                    # a chat
rak "explain what this project does"   # one request
```

**2. Check what is set** with `rak keys`: each provider, whether its key is set, and where it came from (never the key itself).

**3. More keys by hand.** `~/.config/rak/.env` has a line for every provider, the web search key and rak's settings. Open it in any editor, put each key right after its `=` (no quotes):

```bash
ANTHROPIC_API_KEY=sk-ant-…
RAK_PROVIDER=anthropic
RAK_MODEL=claude-sonnet-5-5
```

Scripts and CI never get questions: without a terminal, rak reads the environment and the key file, and says what is missing.

**4. In a project, type `/init`** once. rak looks at the project and writes its notes in `rak/`, so every later session starts already knowing it (see [Project notes](memory-and-skills.md#project-notes-the-rak-folder)).

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
╭─ rak 0.1.0-alpha.15 ─────────────────────────────────────────────────────────╮
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
- **Answers appear as they are written**, with Markdown shown as headings, bullets, code and tables. A table is drawn with aligned columns that fit the terminal, long cells wrapping inside their column.
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

## Command line

| Option | What it does |
|---|---|
| `-m`, `--model NAME` | Model to use. |
| `--provider NAME` | Who runs the model (see [Providers](providers.md)). |
| `--base-url URL` | Another address: a server rak doesn't know by name, or a provider somewhere else. |
| `--api-key-env NAME` | The variable holding the key to send. |
| `--effort LEVEL` | Reasoning effort: `low`, `medium`, `high`, `xhigh` or `max`. |
| `--max-tokens N` | Longest reply to ask for. |
| `--context N` | The model's context window. Default: what Anthropic or Ollama reports, otherwise 128000. |
| `--no-stream` | Wait for each whole reply instead of showing it as it arrives. |
| `-y`, `--yes` | Run every tool without asking. Use it only inside a container. One request with `-y` (an unattended run, as a benchmark or CI does it) keeps nothing for later sessions: no `remember`, skills, routines or journal, so it writes no files of its own into the project. |
| `--read-only` | No edits and no commands: read and report. |
| `--any-path` | Let file tools reach outside the project folder. |
| `--no-mcp`, `--no-skills`, `--no-memory`, `--no-notes` | Leave out MCP servers, skills, memory notes, or the after-request notes update. |
| `--trust` | Use this folder's own `.rak/` settings and `.env` this run, as if trusted (see [Safety](safety.md)). |
| `--tools LIST` | Only these tools, comma-separated (`--tools read_file,search_files,code_map`); MCP tools by their `server__tool` name. `--tools none` sends no tools at all: for a model that can't call tools, or a plain question for less (a one-word question measured 2,232 input tokens with every tool and 256 with none, kimi-k2.6:cloud). A name that matches no tool is said at the start. |
| `-c`, `--continue` | Continue this folder's last chat (`rak -c "and now the tests"`). |
| `-C`, `--cwd DIR` | The project folder. |
| `--max-turns N` | Model calls allowed per request. Default: 30 in a chat (where you can say "go on"), no limit for one request; `0` for no limit. |
| `--helper-budget N` | Input tokens one `delegate` helper may spend. Default 200000; `0` for no cap. |
| `--time-limit SECS` | The time this run has, counted from start. The task tells the model, and it is warned at 70% and 90% to finish the essential work and save its outputs. For benchmarks and CI jobs with a timeout. |
| `--json` | Print one JSON object: answer, outcome, turns, tool calls, tokens, seconds. |
| `--log FILE` | Append every step to FILE as JSON lines. |
| `--record FILE`, `--replay FILE` | Record the model's replies, then play them back offline to rerun a session while fixing rak (a developer tool; SwiftAgentKit's scenario format). |
| `-q`, `--quiet` | Only the answer. |

Other commands: `rak setup`, `rak keys`, `rak skills …`, `rak mcp …`.

Progress goes to stderr, so `rak "…" > answer.txt` holds only the answer. Exit codes: `0` done, `1` failed, `2` bad usage, `3` a tool was not approved, `4` turn limit, `5` stuck repeating a call, `130` stopped with Ctrl-C.
