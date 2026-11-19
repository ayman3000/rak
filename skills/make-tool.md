# Make tool
Description: Make, change or remove a tool of the user's own: when they ask for a tool, a command you can call, or "teach yourself to do X".

A tool of the user's own is one small script. rak offers it to you like its built-in tools, runs the script when you call it, and gives you what it prints. Save tools with `save_tool`; the user sees the whole script and approves it.

## Before you write

1. If the request says what goes in and what comes out, go to "Write it".
2. Otherwise ask, in one short message, only what you're missing: what it takes as input, what it should return, and whether it needs a key or an account.
3. If rak already has a tool that does it (`run_shell`, `fetch_url`, `read_file`, an MCP tool), say so and ask whether they still want their own.

## Write it

- **Language:** `python`, using only Python's standard library (`json`, `sys`, `urllib.request`, `csv`, `re`, `subprocess`). Never a package that needs `pip install`. Use `shell` only for a few lines that pipe standard commands, and `node` only if the user asks.
- **Input:** the arguments arrive as one JSON object on standard input. Read it with `json.load(sys.stdin)`.
- **Output:** print the result as text the next reader can use: plain lines, or JSON for data. Keep it short; print what was asked for, not a log.
- **Failure:** print what went wrong to standard error and exit with a code other than 0. Say what the user or you should do about it.
- **Name:** `lower_case_with_underscores`, a verb and a thing: `csv_to_json`, `search_brave`.
- **Description:** one line that says when to call it. It is all a later session sees before deciding.
- **Parameters:** each with a type (`string`, `integer`, `number`, `boolean`) and a one-line description. Mark the ones it cannot work without as required.
- **`safe`:** true only when the tool changes nothing: it reads, looks up or computes. A tool that writes a file, sends a message or changes anything outside leaves it false, so rak asks before each run.

A whole tool:

```python
import json, sys

args = json.load(sys.stdin)
words = args["text"].split()
print(json.dumps({"words": len(words), "unique": len(set(words))}))
```

## Keys

A tool that calls a service with a key never has the key in its script.

- List the key's name in `needs`, in capitals: `["BRAVE_API_KEY"]`.
- In the script read it from the environment: `os.environ["BRAVE_API_KEY"]`.
- rak gives the script only the keys it lists, from its key file.
- Never ask the user to paste a key in the chat, and never write one into the script. Tell them to type `/key BRAVE_API_KEY`: it is typed hidden and saved for rak only.
- If the key is not set yet, save the tool without `test_input` and tell them the tool works once they have typed `/key NAME`.

## Save it and check it

1. Show the user the script and say in one line what it does and whether it will ask before each run.
2. Call `save_tool`. Pass `test_input` with a small, harmless example so rak tries it once. Leave `test_input` out when a try would change something or needs a key that is not set.
3. If the try failed, read what it said, fix the script and save it again under the same name.
4. Tell the user it is saved, and that it joins the chat with their next message. You cannot call it in the same reply.

## Change or remove one

- To change a tool, save it again under the same name: the new one replaces it.
- To remove one, tell the user to type `/tools remove NAME`. Tools are files in `~/.config/rak/tools`, or in `.rak/tools` for one project.
