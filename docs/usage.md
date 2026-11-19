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

**3. More keys.** `rak keys set openrouter` (any provider's name, or `tavily` for web search) asks for the key, which you paste without it being shown, and saves it; `rak keys set` alone lists the keys to pick from. In a chat, `/key` does the same, and `/provider` asks for the key when the provider you pick has none yet. The key is never an argument, so it does not stay in your shell's history, and replacing one keeps the old file as `.env.bak`.

Or by hand: `~/.config/rak/.env` has a line for every provider, the web search key and rak's settings. Open it in any editor, put each key right after its `=` (no quotes):

```bash
ANTHROPIC_API_KEY=sk-ant-…
RAK_PROVIDER=anthropic
RAK_MODEL=claude-sonnet-5-5
```

Scripts and CI never get questions: without a terminal, rak reads the environment and the key file, and says what is missing.

**4. In a project, say yes to the notes.** The first chat in a folder that has none says "This project has no rak notes yet" and asks "Set them up now? [Y/n]". Enter starts it: rak looks at the project and writes its notes in `rak/`, so every later session starts already knowing it; you approve what it writes. `n` skips it and rak does not ask in that folder again. `/init` does the same any time. The offer is not made in your home folder, in an empty folder, with `-c`, `--read-only` or `--no-notes`, or when nobody is at the terminal.

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
- **Answers appear as they are written**, with Markdown shown as headings, bullets, code and tables, and short formulas of common symbols drawn as characters (`$\rightarrow$` as →, `$n \times m$` as n × m). A table is drawn with aligned columns that fit the terminal, long cells wrapping inside their column.
- **Long lines break at words**, not in the middle of one, and a wrapped bullet continues under its own text. Code is left exactly as written.
- **`Thinking… 12s` while the model works.** When a request has been out for a second and nothing is arriving yet, one dim line counts the seconds; it goes the moment the answer or a tool line appears. A wait of five seconds or more leaves `Thought for 14s` behind. Only on a terminal: never in a pipe, a file or `--json`.
- **Ctrl-C stops a request** (the answer being written, or a running command) and keeps the chat. A second Ctrl-C quits.
- **History** is kept between runs (↑ and ↓). Pasted text keeps its line breaks; a line ending in `\` continues on the next.
- **After each request** a line shows turns, tool calls (a helper's counted apart: `3 tool calls (+14 in helpers)`), tokens in and out, how many came from the cache, and the time.

| Command | What it does |
|---|---|
| `/model [name]` | Show or switch the model. Without a name: pick from a list (↑/↓, Enter, Esc cancels, type to narrow; a name the list doesn't have can be typed and used). |
| `/models` | List the models the provider offers. |
| `/provider [name]` | Show or switch the provider: its own address and key, and the model you last picked there. The conversation comes along, and the next chat in this folder starts there too (kept in `.rak/model`). Without a name: a list that dims providers whose key isn't set. |
| `/add-dir [path]` | Let the agent read and write in another folder, for this chat; edits there ask as in the project. Without a path: the folders it can reach. Your home folder itself and the places keys live are refused. |
| `/key [provider]` | Give or replace an API key from inside the chat: pasted without being shown, checked where the provider has a free check, saved in the key file, and used at once. Without a name: pick from a list. `/provider` asks the same way when the provider you pick has no key yet. |
| `/effort [level]` | Show or switch the reasoning effort for this chat and the next ones in this folder: `low`, `medium`, `high`, `xhigh`, `max`, or `default` (none sent: the model's own). Without a level: pick from a list. A chat starts at `medium` unless `RAK_EFFORT` says otherwise; the status line shows the level being sent. |
| `/init` | Set up this project's notes in `rak/`, or bring them up to date. |
| `/notes` | Show the project's rules and notes (project, roadmap without Done, the latest journal lines). |
| `/tools`, `/mcp`, `/skills` | List the tools and the MCP servers with their tools; `/skills` shows the skills to pick from (↑/↓, Enter, type to filter), then starts your prompt with the picked skill's command so you only type the request (`/skills list` prints them with where each is kept). `/make-tool` (or asking in plain words) has rak write a [tool of your own](tools.md#tools-of-your-own); `/tools remove NAME` deletes one. `/mcp add` adds a server from the JSON on its page, the page's address, or a few questions (it shows it and asks first), usable at once. `/skills add URL` adds a skill (it shows it and asks first), or every skill in a GitHub repository; `/skills remove NAME` deletes one you saved (it asks first). |
| `/SKILL [request]` | Every skill is a command under its own name: `/sdd-plan add a login page` asks the agent to load that skill and follow it for the request. The `/` menu lists them after rak's own commands, and `/help` names them. A name with spaces gets dashes (`/ios-app` for "iOS app"). |
| `/resume [N]` | This folder's saved chats, newest first; `/resume N` picks one up. |
| `/yes` | Turn auto-approve on or off. |
| `/usage` | Tokens, tool calls, the context used and the window, fallbacks. |
| `/jobs` | The servers the agent started in the background (run_shell's background mode; `npm run dev`, `vite`, `flask run` and the like go there by themselves): running or ended, how long, pid, command and log. `/jobs stop N` stops one with everything it started, `/jobs stop all` every running one. When a chat ends with some still running, rak asks whether to stop them; left running, they keep going after rak quits (and in a one-off run they are always left running, so a service stays up for whatever checks it). |
| `/clear` | Start a fresh conversation. The last one stays saved. |
| `/help`, `/exit` | The list of commands; quit (or Ctrl-D). |

## Images

- **Paste an image with Ctrl+V** (or Alt+V, for terminals where Ctrl+V pastes text). The clipboard's image is saved to a file and `[image 1]` (then `[image 2]`, …) goes into the line; the request carries the file's path. On macOS rak reads the clipboard with `osascript`; on Linux with `wl-paste` or `xclip`; on Windows with PowerShell.
- **Drag an image file onto the terminal**, or type its path. The path the terminal pastes is recognised, quoted or with `\ ` for spaces, when it starts at `/`, `~/`, `./` or a drive and ends in `.png`, `.jpg`, `.jpeg`, `.gif` or `.webp`. Up to 6 images per message, 5 MB each. A one-off run from a terminal works the same way: `rak "why does this fail? ~/Desktop/error.png"`. Unattended (no terminal: CI, a benchmark), a path in the task is left as text, and only an explicit `[image: PATH]` is attached.
- **A model that sees** gets the images with your message. For **a model that can't** (DeepSeek, for example), set a vision model in `~/.config/rak/.env`:

  ```
  RAK_VISION_PROVIDER=ollama-cloud     # empty: the same provider as RAK_PROVIDER
  RAK_VISION_MODEL=<a model that reads images>
  ```

  Then rak sends the images and your message to the vision model first (`looking at the image with …`). It is asked to copy every piece of text, code, error and number exactly, and its description goes into your message for the main model, which carries on. The `view_image` tool works the same way, with an optional `question`, so screenshots the agent takes are read too. The vision model's tokens count in the request's totals, and the welcome panel shows it on its `vision` line.
- **Images from MCP tools** (a browser's screenshot, say) are saved to a file in the system's temp folder under `rak-images/`, and the tool's result names it (`[image saved to … (1212×844, 473 KB)]`), so you can open it. The first one also goes to the model the same way: shown to a model that sees, described by the vision model for one that can't, or left out with a note when there is neither.
- **With no vision model set** and a model that can't see, rak says so and how to set one, and sends the message as typed, with the paths in it. Whether a model sees comes from Ollama (it lists each model's capabilities) or, elsewhere, from the model's family: Gemini, GPT-4o and later, the o-series, Claude, Gemma 3 and 4, Llama 4, and VL or vision models see; DeepSeek's text models don't. When rak can't tell, it uses the vision model if one is set, else sends the images and lets the server decide.

## Command line

| Option | What it does |
|---|---|
| `-m`, `--model NAME` | Model to use. |
| `--provider NAME` | Who runs the model (see [Providers](providers.md)). |
| `--base-url URL` | Another address: a server rak doesn't know by name, or a provider somewhere else. |
| `--api-key-env NAME` | The variable holding the key to send. |
| `--effort LEVEL` | Reasoning effort: `low`, `medium`, `high`, `xhigh`, `max`, or `default` (none sent). Wins over `RAK_EFFORT`. Without it a one-off run sends none and a chat starts at `medium`. |
| `--max-tokens N` | Longest reply to ask for. |
| `--context N` | The model's context window. Default: what Anthropic or Ollama reports, otherwise 128000. |
| `--no-stream` | Wait for each whole reply instead of showing it as it arrives. |
| `-y`, `--yes` | Run every tool without asking. Use it only inside a container. One request with `-y` (an unattended run, as a benchmark or CI does it) keeps nothing for later sessions: no `remember`, skills, routines or journal, so it writes no files of its own into the project. |
| `--read-only` | No edits and no commands: read and report. |
| `--add-dir PATH` | Let file tools reach this folder too, besides the project (repeatable). In a chat: `/add-dir PATH`. |
| `--any-path` | Let file tools reach outside the project folder. |
| `--no-mcp`, `--no-skills`, `--no-memory`, `--no-notes` | Leave out MCP servers, skills, memory notes, or the after-request notes update. |
| `--trust` | Use this folder's own `.rak/` settings and `.env` this run, as if trusted (see [Safety](safety.md)). |
| `--tools LIST` | Only these tools, comma-separated (`--tools read_file,search_files,code_map`); MCP tools by their `server__tool` name. `--tools none` sends no tools at all: for a model that can't call tools, or a plain question for less (a one-word question measured 2,232 input tokens with every tool and 256 with none, kimi-k2.6:cloud). A name that matches no tool is said at the start. The list holds for helper agents too: a tool left out is not reachable through `delegate`. |
| `-c`, `--continue` | Continue this folder's last chat (`rak -c "and now the tests"`). |
| `-C`, `--cwd DIR` | The project folder. |
| `--max-turns N` | Model calls allowed per request. Default: 30 in a chat (where you can say "go on"), no limit for one request; `0` for no limit. |
| `--helper-budget N` | Input tokens one `delegate` helper may spend. Default 200000; `0` for no cap. |
| `--time-limit SECS` | The time this run has, counted from start. The task tells the model, and it is warned at 70% and 90% to finish the essential work and save its outputs. For benchmarks and CI jobs with a timeout. |
| `--review` | Before a one-off run (not a chat) that changed something answers, send the model back once to list each requirement, rule and symptom in the request and the docs it points to, quote it, and name the check it ran for it. Skipped with under 15% of `--time-limit` left. Off by default: it costs more model calls (on alpha.19, where it was on, it fixed nothing measured). |
| `--vision-provider P`, `--vision-model M` | The model that reads images for a main model that can't (see Images). Default: `RAK_VISION_PROVIDER` and `RAK_VISION_MODEL`. |
| `--json` | Print one JSON object: answer, outcome, turns, tool calls, tokens, seconds. |
| `--log FILE` | Append every step to FILE as JSON lines. |
| `--record FILE`, `--replay FILE` | Record the model's replies, then play them back offline to rerun a session while fixing rak (a developer tool; SwiftAgentKit's scenario format). |
| `-q`, `--quiet` | Only the answer. |

Other commands: `rak setup`, `rak keys`, `rak keys set NAME`, `rak login chatgpt` and `rak logout chatgpt` (Sign in with ChatGPT, for the `chatgpt` provider; see [providers](providers.md#your-chatgpt-plan)), `rak skills …`, `rak mcp …`, `rak update` (the newest release; `rak update VERSION` for one release, `--check` to only look).

Progress goes to stderr, so `rak "…" > answer.txt` holds only the answer. Exit codes: `0` done, `1` failed, `2` bad usage, `3` a tool was not approved, `4` turn limit, `5` stuck repeating a call, `130` stopped with Ctrl-C.
