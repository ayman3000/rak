# rak — Rust Agent Kit

A small coding agent for the terminal. One binary, no runtime to install, for Linux and macOS.

rak is the cross-platform sibling of [sak](https://github.com/ayman3000/sak). sak is built on SwiftAgentKit and stays the Mac tool; rak carries the same prompt, the same tool designs and the same loop to everything else.

> **Status: early alpha (0.1.0-alpha.7).** The core works and is tested. Much of what sak has is not here yet — see [What is missing](#what-is-missing).

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

On Linux or macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/ayman3000/rak/main/install.sh | sh
```

The script downloads the newest release for your system from rak's GitHub releases, checks it against its published SHA-256 checksum, and puts `rak` in `~/.local/bin`. Nothing else is changed. `RAK_VERSION=v0.1.0-alpha.7` picks a release (the default is the newest, pre-releases included); `RAK_INSTALL_DIR` picks the folder.

Builds: Linux on x86-64 and ARM (static, so they run on any distribution), and macOS on Apple Silicon and Intel. Windows is not supported yet. The macOS builds are not notarized by Apple: installed with the script they run as they are; a build downloaded in a web browser is held back by macOS until you run `xattr -d com.apple.quarantine rak`.

Or build it from source, with Rust 1.88 or newer ([rustup.rs](https://rustup.rs)). This gets `main`, which may be ahead of the newest release:

```bash
cargo install --git https://github.com/ayman3000/rak --locked
```

To update later, run the same command again.

## Getting started

New to rak? Three steps.

**1. Create your key file.**

```bash
rak keys
```

This creates `~/.config/rak/.env` (readable by you only) and lists every provider rak knows, with whether its key is set.

**2. Fill it in.** Open it in any editor:

```bash
open -t ~/.config/rak/.env      # macOS; on Linux: nano ~/.config/rak/.env
```

Put each key right after its `=` (no quotes), and say which provider and model to use:

```bash
# 1. API keys: one per provider. Fill in the ones you have.
OPENAI_API_KEY=
GEMINI_API_KEY=
ANTHROPIC_API_KEY=sk-ant-…
ZAI_API_KEY=
OLLAMA_API_KEY=
OPENROUTER_API_KEY=
…

# 2. Selected provider and model
RAK_PROVIDER=anthropic
RAK_MODEL=claude-sonnet-5-5
```

Leave `RAK_MODEL` empty and rak lists the provider's models to pick from when a chat starts. With Ollama on your machine you need no key at all: leave `RAK_PROVIDER` empty (or set it to `ollama`) and name a model, such as `kimi-k2.6:cloud`.

**3. Check, then start.**

```bash
rak keys
```

Each key you filled in should say `set`, and your provider should be marked `▸`. Then:

```bash
rak                                    # a chat
rak "explain what this project does"   # one request
```

## Upgrading

Already using rak? Update it with the command you installed it with (the install script, or `cargo install --git https://github.com/ayman3000/rak --locked`), then:

```bash
rak keys
```

Your key file is never overwritten. If it was made from an older template, `rak keys` says so. Then:

```bash
rak keys --update
```

It rewrites the file from the current template with each of your values on its line, keeps any value the template has no line for in a section at the end, and saves your old file as `~/.config/rak/.env.bak`. It names what moved and what changed meaning, never a value, and running it again changes nothing. Open the file afterwards to fill in anything new.

What changed meaning with named providers (`--update` warns about the second and third when your file has them):

- **Each provider gets only its own key.** `OPENAI_API_KEY` goes only to OpenAI now, not to any server you name.
- **`RAK_PROVIDER=openai`** means OpenAI itself. If you used it with `RAK_BASE_URL` for another server (OpenRouter, say), set `RAK_PROVIDER` to that server's name and empty `RAK_BASE_URL`.
- **`RAK_API_KEY`** goes only to a server you name with `--base-url`. If it held a provider's key, move it to that provider's line.
- **The last model you picked** is now remembered for each provider. Your earlier one carries over as Ollama's.

## Providers and models

rak knows these providers by name. Each gets its own address and its own key:

| `RAK_PROVIDER` / `--provider` | Who | Key |
|---|---|---|
| `openai` | OpenAI | `OPENAI_API_KEY` |
| `gemini` | Google Gemini | `GEMINI_API_KEY` |
| `anthropic` | Claude through Anthropic's API, with prompt caching | `ANTHROPIC_API_KEY` |
| `zai`, `zai-coding` | Z.ai (GLM); `zai-coding` for GLM Coding Plan keys | `ZAI_API_KEY` |
| `ollama-cloud` | Ollama's cloud at ollama.com, without Ollama installed | `OLLAMA_API_KEY` |
| `openrouter` | OpenRouter | `OPENROUTER_API_KEY` |
| `huggingface` | Hugging Face's router | `HF_TOKEN` |
| `groq` | Groq | `GROQ_API_KEY` |
| `deepseek` | DeepSeek | `DEEPSEEK_API_KEY` |
| `mistral` | Mistral | `MISTRAL_API_KEY` |
| `ollama` (the default) | Ollama on this machine, its `:cloud` models included | none |
| `lmstudio`, `llamacpp`, `vllm` | those servers on this machine | none |

The list is data, [`src/providers.json`](src/providers.json): adding a provider is one line there.

### Tested so far

Which providers have been used for real, against their own servers. The others are set up from each provider's documentation but have not had a real request yet. This table is updated as more are tried.

| Provider | Status | Last tried | Notes |
|---|---|---|---|
| `ollama` | ✅ works | 2026-11-12 | the benchmarks run on it since 2026-11-10 (kimi-k2.6:cloud) |
| `ollama-cloud` | ✅ works | 2026-11-12 | listing models and chatting |
| `anthropic` | ✅ works | 2026-11-12 | prompt caching checked on 2026-11-11 (Claude Haiku 5.5) |
| `openai` | not tried yet | | |
| `gemini` | not tried yet | | |
| `zai` | not tried yet | | |
| `zai-coding` | not tried yet | | |
| `openrouter` | not tried yet | | |
| `huggingface` | not tried yet | | |
| `groq` | not tried yet | | |
| `deepseek` | not tried yet | | |
| `mistral` | not tried yet | | |
| `lmstudio` | not tried yet | | |
| `llamacpp` | not tried yet | | |
| `vllm` | not tried yet | | |

Tried one? Tell us how it went, in an issue.

**Three ways to choose:**

| Where | How | Lasts |
|---|---|---|
| The key file | `RAK_PROVIDER=…` and `RAK_MODEL=…` | until you change it |
| The command line | `rak --provider groq -m MODEL "…"` | that run |
| In a chat | `/provider NAME`, `/model NAME` (`/provider` alone lists them, with whether each key is set) | that chat; the model is remembered |

**Which model a chat starts with:** `-m` if you gave it; otherwise the model you last picked for that provider; otherwise `RAK_MODEL` (it belongs to the provider in `RAK_PROVIDER`, not to one chosen with `--provider`); otherwise rak lists the provider's models to pick from. A one-off run (`rak "…"`) uses `-m`, then `RAK_MODEL`, then your last pick, so scripts stay predictable.

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

### When a provider fails

rak carries on with another provider instead of stopping, with nothing to set up:

1. **The same model is asked again first**, twice, when that can help (network trouble, a busy or rate-limited server). Out of credit, a bad key or a model the server doesn't have move on at once.
2. **Then the next provider**: each other provider whose key is set, in the order the key lines appear in your key file, then the servers on this machine. Each one is tried with **the model you last picked for it**; a provider you have never used in rak is skipped, so a key sitting in the file is never spent on its own.
3. **In a chat, rak asks first**: `ollama-cloud · kimi-k2.6 failed: … Continue with anthropic · claude-sonnet-5-5? [y/n]`. No stops the request. With `-y`, or in a script with no one to ask, it switches and says so.
4. **The step is done again** on the new model, with the conversation so far; an answer cut off part way is redone, not stitched together.
5. **Your next message goes back** to the provider you chose. If that one can't work at all (a bad key, a missing model), the chat stays on the fallback and says why.

**A silent server counts as a failure.** A streamed answer that sends nothing for too long stops and is retried like a network error, instead of waiting out one long limit: 5 minutes for Anthropic (its stream carries pings), 10 for other cloud servers (some think silently), 15 for a server on this machine (it reads a long prompt slowly), each a minute or two longer for a very long prompt. From SwiftAgentKit's `StreamStallPolicy`.

Every switch is shown (`↪ now using …`), logged with `--log`, listed in `--json` (`fell_back_to`) and counted in `/usage`. A helper started by `delegate` falls back the same way. A request rak itself built wrongly, a refusal or a stop does not fall back: another model would not do better.

### Keys and settings

rak reads the environment first, then `./.env` in the project, then `~/.config/rak/.env`; the first one that has a value wins. Values read from the files stay inside rak: commands the agent runs with `run_shell` don't see them. `rak keys` shows what is set and where it comes from, never a key itself; the chat's welcome panel lists the `.env` files it read.

| Setting | For |
|---|---|
| a provider's key (table above) | that provider, and only that provider |
| `TAVILY_API_KEY` | `web_search` |
| `RAK_PROVIDER`, `RAK_MODEL` | the selected provider and model |
| `RAK_BASE_URL` | the selected provider at another address, e.g. Ollama on another machine (optional) |
| `RAK_API_KEY` | the key for a server named only by `--base-url` |

**Set something twice?** The last value set in a file wins, so with two `RAK_PROVIDER` lines the later one is used, and the same for `RAK_MODEL` and the keys. An empty line (the template's own `RAK_PROVIDER=`, say) erases nothing, in the same file or another. `rak keys` and the chat's welcome panel say what was set more than once and which value is used (for a key, only that the last one is used).

**A key never reaches another provider's server.** Each provider is sent only its own key, and defaults from the file never mix with flags: `--provider` on the command line uses that provider's own address even when `RAK_BASE_URL` is set, and `--base-url` on the command line drops the selected provider and its key.

### Claude and prompt caching

An agent sends the whole conversation again with every model call, so most of what it pays for is input it sent before. Prompt caching lets the server read that part back for a fraction of the price: on Anthropic's API a cache read costs a tenth of normal input, or less on the newest models.

With `--provider anthropic` (or a `claude-*` model and `ANTHROPIC_API_KEY` set), rak speaks Anthropic's Messages API directly for this, rather than going through its OpenAI-compatible endpoint:

- **Caching** uses two markers: one on the system prompt (which covers the tools too, since they come first), and the automatic one that follows the end of the conversation. Each call then reads back everything the previous call sent.
- **Thinking blocks** are sent back unchanged with each request, as the API requires.
- **Refusals.** For the models that can decline a request (Claude Fable 5.2 and 5.1, Opus 5.5 and 5, Sonnet 5.5), rak asks for Anthropic's server-side fallback: on a decline, the API answers with the model it recommends in the same call. A decline with no fallback ends the request and says why.
- **The context window** is read from Anthropic's models list.

After each request rak shows how much of the input came from the cache (`6220 tokens in (3000 cached)`), and `/usage` and `--json` include it. Servers on the chat-completions API that report cached tokens show them the same way; Ollama's cloud models do.

## Chat

Started without a task, rak opens a chat with the same interface as sak:

```
╭─ rak 0.1.0-alpha.7 ───────────────────────────────────────────────────────────────────────╮
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
- **No model named?** rak uses the one you last picked for that provider, then `RAK_MODEL`, or lists the provider's models to pick from.
- **Pick from a list:** `/provider` and `/model` open a list: ↑ and ↓ move, Enter picks, Esc cancels and keeps what you had, and typing narrows the list (for models, a name the list doesn't have can be typed and used). Providers whose key isn't set are shown dim.
- **Switch provider mid-chat** with `/provider NAME`: its own address and key, and the model you last picked there. The conversation comes along.

| Command | What it does |
|---|---|
| `/model [name]` | Show or switch the model. Without a name: pick from a list (↑/↓, Enter, Esc cancels, type to narrow). |
| `/models` | List the models the provider offers. |
| `/provider [name]` | Show or switch the provider. Without a name: pick from a list that says whether each one's key is set. |
| `/tools` | List the tools the agent can use. |
| `/mcp` | List the MCP servers and their tools. |
| `/skills` | List the skills the agent can load. `/skills add URL` adds one (it shows it and asks first). |
| `/notes` | Show the project's rules and its notes in `rak/` (project, roadmap without Done, the latest journal lines). |
| `/yes` | Turn auto-approve on or off. |
| `/usage` | Tokens and tool calls so far, and the context window. |
| `/clear` | Start a fresh conversation. The last one stays saved. |
| `/resume` | This folder's saved chats, newest first, with when and how long. `/resume N` picks one up. |
| `/help`, `/exit` | The list of commands; quit. |

The history and the last model picked for each provider are kept in `~/.config/rak/`. In a chat, your last pick for a provider wins over `RAK_MODEL`; `RAK_MODEL` is where a provider you haven't picked a model on yet starts, and it applies only to the default provider. A one-off run (`rak "…"`) uses `-m`, then `RAK_MODEL`, then your last pick, so scripts stay predictable.

| Option | What it does |
|---|---|
| `-m`, `--model NAME` | Model to use; wins over your last pick and `RAK_MODEL`. A chat can start without one. |
| `--provider NAME` | Who runs the model (table above). Or set `RAK_PROVIDER`. Default: `ollama`, or `anthropic` for a `claude-*` model when `ANTHROPIC_API_KEY` is set. |
| `--base-url URL` | Another address: a server rak doesn't know by name, or a provider somewhere else. Or set `RAK_BASE_URL`. |
| `--api-key-env NAME` | The variable holding the key to send. Default: the provider's own key; `RAK_API_KEY` for a server named by `--base-url`. |
| `--effort LEVEL` | Reasoning effort: `low`, `medium`, `high`, `xhigh` or `max`. |
| `--max-tokens N` | Longest reply to ask for. |
| `--context N` | The model's context window in tokens. Default: what Anthropic or an Ollama server reports for the model, otherwise 128000. |
| `--no-stream` | Wait for each whole reply instead of showing it as it arrives. |
| `-y`, `--yes` | Run every tool without asking. |
| `--read-only` | No edits and no commands: read and report. |
| `--any-path` | Let file tools reach outside the project folder. |
| `--no-mcp` | Don't connect the MCP servers for this run. |
| `--no-skills` | No skills for this run, built-in ones included. |
| `--no-memory` | No notes from earlier sessions, and no `remember`. |
| `--no-notes` | Don't update `rak/journal.md` and `rak/roadmap.md` after a request. |
| `-c`, `--continue` | Continue this folder's last chat, in a chat or with one request (`rak -c "and now the tests"`). |
| `-C`, `--cwd DIR` | The project folder. |
| `--max-turns N` | Model calls allowed per request. Default 30. |
| `--json` | Print one JSON object: answer, outcome, turns, tokens (cached ones too), seconds. |
| `--log FILE` | Append every step to FILE as JSON lines. |
| `--record FILE` | Write the model's replies to FILE as they arrive (a developer flag). |
| `--replay FILE` | Play recorded replies instead of asking a model: the same session again, offline, as often as needed while fixing rak around it. The token counts shown are the recorded ones. Same JSON shape as SwiftAgentKit's replay scenarios. |
| `-q`, `--quiet` | Only the answer. |

On a terminal the answer is shown as it arrives. Progress lines go to stderr, so `rak "…" > answer.txt` holds only the answer.

Exit codes: `0` done, `1` failed, `2` bad usage, `3` a tool was not approved, `4` turn limit, `5` stuck repeating a call, `130` stopped with Ctrl-C.

## Safety

- **Edits and commands ask first.** `write_file`, `edit_file`, `apply_patch`, `run_shell`, `save_skill`, `remember` and `save_routine` need a yes each time, unless you answer "always" or start with `-y`.
- **A no stops the request.** The model does not get another turn to try the same change a different way.
- **No terminal, no changes.** In a script or CI without `-y`, rak stops with exit code 3 instead of guessing.
- **A tool error is not passed off as done.** When the step just before the model's answer had a tool error (an edit that didn't apply, a rejected write), rak asks once: fix it, or say plainly what failed. From SwiftAgentKit's repair retry; at most three times a request. A command that exits with an error is a normal result, not a tool error.
- **File tools stay in the project folder** unless you pass `--any-path`. Symlinks are resolved before the check.
- **`run_shell` is not confined.** A command you approve can do anything your user can. The approval prompt shows the whole command, every line of it: read it before you say yes.
- **An MCP server is a program you chose to run.** rak starts it with your user's rights, and trusts what it says about which of its tools are read-only.

## Tools

| Tool | What it does |
|---|---|
| `list_dir` | Lists a folder. |
| `search_files` | Finds files by name and by text inside them. Skips `.git`, `node_modules`, `target`, `.build`. |
| `code_map` | Maps the project: every source file with the functions, types and classes it defines, and their lines. Rust, Python, JavaScript/TypeScript, Swift, Go, Kotlin, Java, C#, Dart, Ruby and PHP; found line by line, without a parser, so an oddly written definition can be missed. Skips dependency, build and hidden folders. Source folders (`src`, `lib`, `app`…) come first; benchmarks, examples, tests and docs last. Stops at about 14,000 characters and then names every folder with files it didn't map, so the agent maps those with `path` instead of guessing. |
| `read_file` | Reads a file. Files up to 40,000 characters come back in one call; small `limit`s are widened, because many small reads cost more than one large one. `start_line` and `end_line` read just part of a file, such as a function `code_map` found. |
| `view_image` | Shows the model an image file (PNG, JPEG, GIF, WebP), such as a screenshot it took, so it can check what a screen really shows. Offered only when the model can see: Claude always; on Ollama, models that list "vision" (gemma4 and kimi-k2.6 do, glm-5.2 does not); on other servers, offered and left to the server. Images count as about 1,600 tokens each against the context window, and old ones are dropped first when it fills. |
| `write_file` | Writes or appends a file. Refuses to overwrite a file that changed on disk since rak last read it. Checked before writing (below). |
| `edit_file` | Replaces exact text. Tolerates trailing-whitespace differences; when the text is not found it shows the file's lines near the closest match. Checked before writing (below). |
| `apply_patch` | Applies a unified diff to one file: several changes in one approved step. Hunks are placed by their lines, not their `@@` numbers; trailing whitespace and up to two edge context lines may drift, never a `-` or `+` line. All or nothing: a hunk that fails changes nothing and the error shows the file's real lines there. From SwiftAgentKit's `PatchFileTool`. |
| `run_shell` | Runs a command with a time limit that kills the whole process tree. Background mode for servers. |
| `todo` | The agent's task list for work with three or more steps, shown to you each time it changes. If the agent says it is done while steps it listed are still open, rak sends it back once to finish them or say why not. The list is never dropped when the context fills. |
| `delegate` | Hands a question to a helper agent with a fresh context and read-only tools (`list_dir`, `search_files`, `code_map`, `read_file`, `fetch_url`, and `web_search` when there is a key); only its answer comes back. The helper's steps are shown indented, it uses the chat's current model, its tokens count in the request's totals, and it cannot delegate further. |
| `web_search` | Searches the web through [Tavily](https://tavily.com): the most relevant pages with a short extract of each, and a direct answer when there is one. Only there when `TAVILY_API_KEY` is set; it only reads, so it needs no approval and stays in read-only mode. Each search spends a Tavily credit (`deep` spends two). |
| `fetch_url` | Reads one public http(s) page as Markdown (headings, lists and links kept; scripts, styles and chrome dropped), as text, or as raw HTML; capped at 40,000 characters unless `max_chars` says otherwise. Only reads, so no approval. Addresses on this machine or a private network are refused, checked in the resolver the connection itself uses, so a redirect or a changed DNS answer cannot reach them either. From SwiftAgentKit's `FetchURLTool` and `SSRFGuard`. |
| `remember` | Keeps a note for later sessions — how you like things done, a lasting decision or convention — in `rak/memory.md` (this project; alpha.6's `.rak/memory.md` is still read) or `~/.config/rak/memory.md` (every project), one line each. Asks first and shows the note whole: a note steers every later session. A session reads both files when it starts and puts the notes in its instructions, marked as possibly out of date; the welcome box counts them. Edit or delete the files freely. From SwiftAgentKit's `remember`, without its separate filing step. |
| `save_routine`, `run_routine` | A routine is steps that just worked, saved so they run back to back with no model call between them: a task that took a dozen model steps takes one. `save_routine` keeps them, with `{placeholders}` for what varies, in `.rak/routines/` (this project) or `~/.config/rak/routines/` (every project); it asks first and shows every step. `run_routine` appears once a routine exists and lists them; each step still asks for approval like the model's own call, and the first failure stops the routine and says which step. Same JSON shape as SwiftAgentKit's, so sak and Naseem read them too. |

**The write check** (from SwiftAgentKit's `WriteVerifier`). Before `write_file`, `edit_file` or `apply_patch` changes a file, the file as it would be afterwards is checked; if the check fails, the file is left as it was and the model is told why.

- **Damaged content:** many lines starting with a diff's `+` in a code file means patch text leaked into the file. The model is told to resend.
- **Syntax:** JSON is parsed by rak itself; files that by convention allow comments (`tsconfig.json`, `.vscode/*.json`, `.jsonc`) may have them. Python, JavaScript, shell scripts, Swift and Dart go to their own parser (`python3`, `node --check`, `bash -n`, `swiftc -parse`, `dart format`). The model is told resending the same content will fail the same way, and which interpreter judged it.
- **When a machine has several copies of an interpreter, the newest judges,** so an old system Python does not reject code written for the one in use.
- **A check that cannot run passes.** No parser installed, a file over 2 MB, a parser that hangs past 10 seconds or crashes: the write goes ahead. A false rejection would block the agent, which is worse than a miss.

## Skills

A skill is know-how written in Markdown: how to build an iOS app from the terminal, how this team deploys, how a library is meant to be used. The system prompt lists each skill as one line, its name and a description, and the agent loads a skill's full instructions with `use_skill` when a task matches. A skill costs a line in the prompt until it is used.

rak reads skills from `.rak/skills/` in the project, then from `~/.config/rak/skills/`; a project's skill wins over one with the same name. Two formats work:

- **`NAME.md`**, as in SwiftAgentKit and sak: a `# Name` line, a `Description:` line, then the instructions. Files the skill ships (templates, references, scripts) go in a folder `NAME/` beside it.
- **A folder with a `SKILL.md`**, as in Claude's Agent Skills: a `---` block with `name:` and `description:`, then the instructions. The folder holds the skill's files.

The model reads a skill's files only when the instructions point it at one. In a chat, `/skills` lists the skills and where each came from.

### Writing skills with the agent

The agent can write skills: `save_skill` saves one (a `NAME.md` in SwiftAgentKit's format) to `~/.config/rak/skills/` or, for this project only, to `.rak/skills/`. It works the way SwiftAgentKit's `learn_skill` does, with one difference: the user approves every save, because a skill is instructions every later session follows. The approval shows the whole skill. A saved skill can be loaded at once; the skills index in the prompt lists it from the next session, so the prompt stays the same within a session and stays cached.

- **When the user asks for a skill,** the agent loads the built-in skill `Write a skill` and guides them. It asks only what it is missing (the task, how it is done now, how the result is checked, what to avoid, which projects it is for), shows the draft, and saves it once they agree.
- **After a mistake,** the agent keeps the lesson. The system prompt tells it to save one when the user corrects it, or when something worked only after failed tries. And when the user says no to a tool call, the model is reminded to save what the user says next.

Tried with kimi-k2.6:cloud (2026-11-11), in a test folder:

- **Asked for "a skill for writing git commit messages the way my team does":** it loaded `Write a skill` and asked three questions. After the answers it showed a draft, saved it when told to, and the file was there.
- **Corrected after recovering on its own:** told to run `python3 test_app.py`, it saw the error and switched to `./check.sh`. The user then said "Next time don't run the file directly: in this project tests always go through `./check.sh`, because it sets APP_ENV first". It saved a project skill holding the rule and the reason. A new session asked to "run the tests" loaded that skill and ran `./check.sh` first.
- **Corrected after a refusal:** the user said no to `APP_ENV=test python3 test_app.py` and wrote "No. In this project tests always go through ./check.sh…". In three tries, each with a different wording of the prompt line, it ran `./check.sh` but kept nothing. With the reminder added to what the model is told after a no, the same exchange made it save the lesson unasked (one try).
- **A plain correction with no refusal and no "next time"** was tried once, before the prompt line existed, and nothing was kept.

These are a handful of tries with one model, not a measurement. A lesson was kept when the user said no to a step or said the correction was for next time. If a correction matters, you can also just ask the agent to save it.

### Adding skills from elsewhere

```bash
rak skills add https://github.com/anthropics/skills/tree/main/skills/pdf
rak skills add ./deploy.md --project
rak skills                    # list them
rak skills remove pdf
```

`rak skills add` takes a web address or a file. For GitHub, a file's page or a skill folder's page both work; for a folder, it also downloads the files the skill ships (scripts, references), through GitHub's API. Before saving it shows the skill and its files, and points out lines worth reading first: commands piped into a shell, deletions, `~/.ssh`, `.env`, and text telling the agent to ignore the user. That list is a hint, not a verdict: read the skill. Saving asks unless you pass `-y`. In a chat, `/skills add URL` does the same, and the agent can load the skill straight away. Like SwiftAgentKit's `import_skill`, nothing fetched is trusted before a person has seen it.

**Built-in skills.** rak ships with skills of its own, compiled into the binary from the repository's [`skills/`](skills) folder, and offers each only on the systems it applies to. A skill of yours with the same name replaces a built-in one, and `--no-skills` leaves all skills out of a run. `/skills` says which are built in.

| Skill | Offered on | What it does |
|---|---|---|
| `iOS app` | macOS | Builds an iOS app from the terminal and runs it in the Simulator (below) |
| `Write a skill` | all | How to write a good skill, guide the user through one, keep a lesson, and review one from a web address |

Skills cost input tokens on every model call, mostly read back from the server's cache. Measured with kimi-k2.6:cloud on a Mac, asking for one word: 1,157 tokens with `--no-skills` and 1,459 with skills, so 302 more. That covers both built-in skills' index lines, the `use_skill` and `save_skill` tools, and the line about keeping lessons. `--read-only` leaves out `save_skill` and that line.

[`skills/ios-app.md`](skills/ios-app.md) builds an iOS app from the terminal and runs it in the Simulator, without an Xcode project: `swiftc` for the Simulator, `simctl` to install, launch and screenshot it, and a Swift package for logic tests. Without it, gemma4:31b-cloud asked to "create an Xcode weather project with one nice screen and test it" invented a command that does not exist (`xcodebuild -create-project`), then gave up on the Simulator and told the user to finish in Xcode. With it, the same model built the app, fixed its own compile error, launched it in the Simulator, ran its test, and reported which checks it made.

A second session, after the skill learned launch arguments and the app's log, and with `view_image`, asked the same model for a weather app for Egyptian cities with a Settings tab, using the free Open-Meteo API, tested on the Simulator. Before writing any code the model called the API with `curl` for three cities. It tested the decoding with `swift test`, opened the Settings screen through a launch argument (the app still starts on its main screen), and looked at both screenshots with `view_image`; its descriptions matched what was on screen. One claim went too far: it reported the fetches as verified from a log that printed "Verified" even when a fetch failed, from test code that ran on every launch. The skill now forbids both.

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
- **A server costs tokens on every request**, because its tool descriptions are sent each time. The reference filesystem server's 14 tools added 1,165 tokens to each model call when measured (2,187 against 1,022). So a large server — tool descriptions over 3,000 characters, about 1,000 tokens — is **loaded when needed**, as in SwiftAgentKit: the system prompt names it in one line, and its tools are sent only after the model calls `load_tools` for it, for the rest of that chat. Loading changes the tool list, so a provider's prompt cache starts again from that call. Smaller servers are always sent. Add the servers you use.

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

## Project notes: the `rak/` folder

As Naseem does with its `naseem/` folder, rak keeps a project's notes in a visible `rak/` folder inside the project — plain Markdown to read, edit and commit:

| File | What it holds |
|---|---|
| `rak/project.md` | What the project is, who it is for, the stack, how to build and test. |
| `rak/roadmap.md` | `## Done`, `## In progress`, `## Next`, `## Known issues`. |
| `rak/journal.md` | One dated line per request that changed something: `- 2026-11-13 20:51 [fix] — asked → what changed`. Past 30 lines the oldest move to `journal-archive.md`; nothing is deleted. |
| `rak/rules.md` | Your instructions for this project (read with `AGENTS.md` and `CLAUDE.md`), with an optional `## Gate`. |
| `rak/memory.md` | Notes kept with `remember`. |

- **Ask to start them.** "Set up the rak notes for this project" writes `project.md` and `roadmap.md`. Without them, rak keeps no journal.
- **Loaded, not read.** Every session starts with the rules, `project.md` and the roadmap's working sections (Done left out) in its instructions, so it starts oriented instead of spending its first steps reading. The welcome box shows `rules` and `notes`.
- **Kept current.** After a request that changed a file or ran a command, one small model call writes the journal line and names roadmap edits — move a card, add a card. rak applies them to the file as it is: a card it can't find is skipped and said so, and nothing is ever deleted (Naseem's lesson: a model asked to rewrite a roadmap dropped 8 known issues). Cards moved to In progress or Done get `started:` / `done:` dates. A question costs nothing extra; `--no-notes` turns the update off, and read-only mode never writes.
- **The gate.** A `## Gate` section in `rak/rules.md` (or `AGENTS.md`) with a fenced command:

  ````markdown
  ## Gate
  ```
  cargo test && cargo clippy -- -D warnings
  ```
  ````

  A request that edited files is not done until every part of that command has run after the last edit: if the agent says "done" without running it, rak sends it back (twice at most) to run it and report the real output.

## Large projects

What helps the agent on a project too big to read whole:

- **Chats are saved** after every request, one file per chat in `~/.config/rak/sessions/`, 30 kept per project folder. `rak -c` continues the last one and `/resume` picks any. The system prompt is not saved: a resumed chat gets today's, with today's skills and notes.
- **A map first.** `code_map` shows where everything is, and `read_file` with `start_line`/`end_line` reads only those lines.
- **A plan it keeps.** `todo` holds the steps; rak checks them when the agent says it is done.
- **A helper for searching.** `delegate` reads many files in its own context and returns only the answer.
- **Memory across sessions.** The project's rules, its notes in `rak/` (below), `remember` notes and project skills (`.rak/skills`) carry conventions and lessons into every chat.
- **When the context fills,** old tool outputs are replaced by a note saying which call they came from (`read_file src/main.rs`) and how they began, so the agent knows what it no longer has. The output itself is kept aside for the chat: the note names it (`out-3`), and the agent reads it back with `artifact_read` or finds lines in it with `artifact_search` instead of running the tool again (from SwiftAgentKit's artifact store; those two tools join the list only once something is kept, and a resumed chat doesn't have them). The tool descriptions count toward the window too.
- **When that is not enough,** the older part of the conversation is replaced by a summary the model writes, and the newest turns stay word for word, the latest request always among them (as SwiftAgentKit's compaction does). The task list is copied into the summary unchanged. If the summary call fails, a plain list of the user's messages and the calls made stands in. Only after that are recent tool outputs replaced by notes too.
- **When the server says the request is too long** (its token count can differ from rak's estimate), rak summarizes once more and asks again. If there is nothing older to summarize and nothing to trim, the request fails and says the latest message or output alone is too big.

A line in the system prompt says when to use these. The four tools add 214 input tokens to every call (1,157 → 1,371 with skills off, kimi-k2.6:cloud).

Tried with kimi-k2.6:cloud (2026-11-11):

- **A multi-file build:** a library CLI in Python, with models, JSON storage, argparse commands and tests, done with `-y` in the benchmark's container. It kept a five-step task list from start to finish, ran its tests, and ran the CLI end to end. I ran its 7 tests again afterwards: they pass. Its first `todo` call named the field `name` instead of `text`; the tool now takes either.
- **A question about rak's own code** (about 45 files): "every condition under which this program talks to Anthropic's API, with file:line". Without the system-prompt line, it never used `code_map` (128,000 and 185,000 input tokens in two runs). With it, it started with `code_map` each time; runs took 88,000, 136,000 and 211,000 tokens, so the model's own choices vary more than the tools change. The answers were right every time.
- **`delegate`:** kimi did not choose it on its own, even for "summarize how every tool checks its inputs" (13 files read in its own context). Asked to use it, the helper found the right file; one line number was exact and one was off by five. The main conversation spent three turns.

Compaction was tried with kimi-k2.6:cloud (2026-11-12) by forcing a tiny window (`--context 4000`) on a read-only chat about rak's own files. A second request summarized the first ("summarizing 7 earlier messages"), and the model still named the three files the first request was about. In the first request there was nothing older to summarize, so the last-resort step replaced the file just read with its note; the model answered about that file from the note instead of reading it again, and got it wrong. With real windows (128K and up) a single read of at most 40,000 characters does not reach that step.

Not tried yet: a real codebase of thousands of files, a real session long enough to fill a full-size window, and `delegate` with Claude.

## What is missing

Compared with sak, rak does not yet have:

- Ctrl-R to search the history, and the "still running" note during a long tool call
- Gemini's own API (`--provider gemini` uses its OpenAI-compatible endpoint)
- Naseem's notes and skills (rak reads its own skills, in the same format)

Known limits of the chat:

- Ctrl-C takes effect when the model next sends something. While a model is silent (still thinking), the first Ctrl-C waits; a second one quits rak.
- On Windows the chat falls back to plain line input: no menu, no arrow keys, no history.
- The window rak shows for a local Ollama model is the model's own maximum. Ollama may run it with a smaller one.
- With Anthropic's API, trimming old tool output or summarizing near a full context window changes earlier messages, which ends the cache from that point; the next call writes it again. A summary is one more model call, counted in the request's tokens.

Not yet tested: Windows (not built yet), and any public benchmark. Anthropic's real API was tried on Claude Haiku 5.5 (2026-11-11): the first call wrote 1,514 tokens to the cache and the second read all 1,514 back, writing only the 87 new ones. Its other paths (thinking blocks sent back, refusal fallbacks) are tested against a local stand-in. The chat and the MCP client were driven by hand on macOS only; on Linux they build and their unit tests pass. The MCP client was tried with one real server (the reference filesystem server, a program); web-address servers are covered by a unit test against a stand-in. Auto mode (`-y`) has only been run inside the benchmark's containers.

## Development

```bash
cargo test
```

The code is in two layers. The library knows nothing about terminals; the `rak` command is one front end on top of it, and another (an editor plugin, a web server) could use the library the same way.

| Library (`src/`) | |
|---|---|
| `agent.rs` | The loop: ask the model, run the tools it calls, repeat. Retries, loop detection, context trimming, the finish check. |
| `compact.rs` | Compaction: where the summarized part ends, what the summary is written from, how it rejoins the conversation, which server errors mean "too long". |
| `sessions.rs` | Saved chats: one JSON file per chat, per project folder. |
| `delegate.rs` | The `delegate` tool: a helper agent run inside a tool call. |
| `provider.rs` | The `Model` trait and the OpenAI-compatible client, streaming included. |
| `anthropic.rs` | Claude through Anthropic's Messages API: caching, thinking blocks, refusal fallbacks. |
| `providers.rs`, `providers.json` | The providers rak knows by name: address, key, API. |
| `tools/` | The `Tool` trait and the built-in tools (`fs`, `shell`, `map`, `todo`, `image`, `web`); `Ctx` holds what they share; `verify` is the write check. |
| `mcp/` | `config` (mcp.json), `client` (one server connection), and the bridge that turns a server's tools into `Tool`s. |
| `skills/` | Reading skills, the index and `use_skill`; `save` writes and removes them (`save_skill`); `fetch` gets one from a web address. |
| `interrupt.rs`, `read_slice.rs`, `text.rs` | The stop flag, the file-read window, small text helpers. |

| Command (`src/cli/`) | |
|---|---|
| `args`, `session` | The command line; one run set up from it, and a request run and reported. |
| `chat` | The chat loop and its slash commands. |
| `approval`, `output`, `log` | Asking before changes; what is shown; `--log`. |
| `mcp_command`, `skills_command`, `keys_command` | `rak mcp add/list/remove`, `rak skills list/add/remove`, `rak keys`. |
| `editor`, `menu`, `markdown`, `welcome`, `style`, `term` | The interface's parts. Each is pure where it can be, and tested. |
| `settings`, `signals` | What is kept between runs; Ctrl-C. |

**Checks and releases.** Every push to `main` and every pull request runs the tests on Linux and macOS, the formatting and lint checks, and a build with the oldest Rust rak supports (`.github/workflows/ci.yml`). A version tag that matches `Cargo.toml` builds the four release archives and publishes them (`.github/workflows/release.yml`):

```bash
git tag v0.1.0-alpha.7 && git push origin v0.1.0-alpha.7
```

A tag with a `-` makes a pre-release.

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
| 2026-11-11 | with the honesty rule (first run) | 9/12 | 394 s | 185K (13.8K), 157K of it cached | 34K |
| 2026-11-11 | with the honesty rule (second run) | 12/12 | 521 s | 183K (15.5K), 158K of it cached | 53K |

All with kimi-k2.6:cloud through Ollama, two attempts per task. The tool descriptions were the same in every run; the system prompt too, until the last two runs added the honesty rule.

The honesty rule (report only checks that actually ran, and say what was not checked) was run twice because its first run scored 9/12. The three misses were the model's judgement, not missed checks: in each it had run its own tests. Task 3 reused a deleted to-do's number, task 4 refused the discount code `flat5`, and task 6 fell into the `"5 5"` trap. The second run scored 12/12. Together that is 21 of 24, in line with the runs before, with the same input per run.

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
