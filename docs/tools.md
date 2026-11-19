# Tools and large projects

What the agent can do, how every write is checked, and how rak works on a project too big to read whole. Back to the [README](../README.md).

**Contents:** [Tools](#tools) · [Tools of your own](#tools-of-your-own) · [Every write is checked first](#every-write-is-checked-first) · [Large projects](#large-projects)

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
| `run_shell` | Runs a command with a time limit that kills the whole process tree. Background mode for servers. Long output keeps its start and its end, where a failing build prints its error, and says how much of the middle was cut. | yes |
| `todo` | The agent's task list for work with three or more steps, shown to you as it changes. If the agent says it is done with steps still open, rak sends it back once. | |
| `delegate` | Hands a question to a helper agent with a fresh context. The helper has only `list_dir`, `search_files`, `code_map`, `read_file`, `fetch_url` (and `web_search` with a key): no shell, no edits. Only its answer comes back, so a big search doesn't fill the main context. Several `delegate` calls in one reply run at the same time: up to 3 on a commercial cloud API, 2 on Ollama's cloud, 1 on a model on this machine (the rest wait their turn). Their steps show indented, tagged `[1]`, `[2]`… Each helper may spend 200,000 input tokens (`--helper-budget`); near that it is told to wrap up, and at it to answer with what it found. | |
| `web_search` | Searches the web through [Tavily](https://tavily.com). Only there when `TAVILY_API_KEY` is set; each search spends a Tavily credit. | |
| `fetch_url` | Reads one public web page as Markdown, text or HTML. Addresses on this machine or a private network are refused, even through a redirect. | |
| `remember` | Keeps a one-line note for later sessions, for this project or for all. | yes |
| `past_chats` | In a chat: this project's earlier conversations, so "what did I ask you first?" has an answer. Lists them, reads one (your requests and rak's answers; tool output is left out), or searches them all. Read-only, this project only, the current chat left out; nothing is sent to the model unless it calls the tool. A one-off run does not have it. |
| `save_tool` | Saves a [tool of your own](#tools-of-your-own): one script, shown whole first. | yes |
| `save_routine`, `run_routine` | Saves steps that just worked, with `{placeholders}`, so they run again back to back with no model calls between them. Each step still asks like the model's own call. Same format as sak and Naseem. | yes |

Two more appear only when needed: `load_tools` for a [large MCP server](mcp.md), and `artifact_read` / `artifact_search` once [old output has been set aside](#large-projects).

## Tools of your own

Ask for a tool in plain words ("I need a tool that converts CSV to JSON") or type `/make-tool`. rak writes one small script, shows it to you whole, and asks before saving it:

```
? allow save_tool "word_count" (python) for every project:
    Count the total words and unique words in a given text.
    runs without asking (marked as changing nothing)
    and is run once now with {"text":"Hello world hello"}
    │ import json, sys, re
    │ args = json.load(sys.stdin)
    │ words = re.findall(r"\b\w+\b", args.get("text", "").lower())
    │ print(json.dumps({"total_words": len(words), "unique_words": len(set(words))}))
  [y]es  [n]o  …
```

On yes the tool is saved, tried once, and joins the chat with your next message. From then on every chat has it.

- **One file per tool,** in `~/.config/rak/tools` (every project) or `./.rak/tools` (one project; used once you trust the folder). rak runs the file when the tool is called: nothing starts with rak, and a broken tool breaks only itself. `rak update` leaves them alone.
- **Languages:** Python with its standard library only (the default), a shell script, or Node. Nothing to compile and nothing to `pip install`. The tool reads its arguments as one JSON object on its input and prints its result.
- **Asking:** a tool marked `safe` only reads or computes, and runs freely. Any other asks before each run, like an edit or a command. `--read-only` keeps only the safe ones.
- **Keys:** a tool that calls a service names the key it needs (`# needs: BRAVE_API_KEY`) and reads it from its environment. The key is never in the script. Give it once with `/key BRAVE_API_KEY` (typed hidden, saved in rak's key file). rak hands a tool only the keys it names, takes other secrets out of its environment, and hides the key if the tool prints it. A tool whose key is missing does not run, and rak says which key to give.
- **`/tools`** lists your own after rak's; **`/tools remove NAME`** deletes one. To change one, ask for the change: the new version replaces it.
- **By hand:** a tool is a script with a short header, so you can write or edit one yourself.

```python
#!/usr/bin/env python3
# rak-tool: csv_to_json
# description: Convert a CSV file to JSON.
# param: path (string, required) The CSV file.
# param: indent (integer) Spaces of indentation.
# needs: BRAVE_API_KEY
# safe: yes
# timeout: 30

import csv, json, sys
args = json.load(sys.stdin)
print(json.dumps(list(csv.DictReader(open(args["path"]))), indent=args.get("indent")))
```

These tools work in rak only. For a tool other clients can use too, write an [MCP server](mcp.md).

A script is a program on your machine: it can read your files like any program you run. Read it before you say yes.

## Every write is checked first

From SwiftAgentKit's `WriteVerifier`. Before `write_file`, `edit_file` or `apply_patch` changes a file, the new content is checked, and if the check fails the file is left as it was and the model is told why:

- **Leaked patch text:** many lines starting with a diff's `+` in a code file.
- **Syntax:** JSON is parsed by rak itself (comments allowed where the convention allows them). Python, JavaScript, shell scripts, Swift and Dart go to their own parser; when several copies are installed, the newest judges.
- **A check that cannot run passes** (no parser, a file over 2 MB, a parser that hangs past 10 seconds), because a false rejection would block the agent.

## Large projects

What helps the agent on a project too big to read whole:

- **A map first.** `code_map` shows where everything is, and `read_file` with line numbers reads only what's needed.
- **A plan it keeps.** `todo` holds the steps, and is never dropped when the context fills.
- **Helpers for searching.** `delegate` reads many files in its own context and returns only the answer; independent questions run side by side.
- **Chats are saved** after every request (30 per project folder). `rak -c` continues the last one; `/resume` picks any; and the model can look into them itself with `past_chats`.
- **When the context fills,** old tool outputs are replaced by a short note naming the call (`read_file src/main.rs`). The output itself is set aside (`out-3`), and the agent reads it back with `artifact_read` or searches it with `artifact_search` instead of running the tool again.
- **When that isn't enough,** the older part of the conversation is replaced by a summary the model writes; the newest turns and the task list stay word for word. If the server still says the request is too long, rak summarizes once more and asks again.
