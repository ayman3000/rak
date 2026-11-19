# MCP servers

Tools from outside rak come through the [Model Context Protocol](https://modelcontextprotocol.io). A server is a separate program or a web address; its tools join rak's own. Back to the [README](../README.md).

## Adding one

Most servers' pages show a block of JSON to "add to your MCP client". rak takes it in whichever way is easiest:

```bash
rak mcp add                                                          # paste the JSON, or the page's address, or answer questions
rak mcp add https://github.com/ChromeDevTools/chrome-devtools-mcp    # read it from the page
rak mcp add '{ "mcpServers": { "files": { "command": "npx", "args": ["-y", "fs-mcp", "."] } } }'
```

Whichever you use, rak shows what it found and asks before it saves, because a server is a program that runs on your machine:

```
From https://github.com/ChromeDevTools/chrome-devtools-mcp:
  chrome-devtools
    runs:  npx -y chrome-devtools-mcp@latest
This is a program rak will start on your machine. It is downloaded when it starts, so what runs is whatever its publisher last released.
Add it? y: for all projects · p: this project only · n: no ›
```

- **A page with several configs** (one per client, or with options) lists them numbered for you to pick. Repeats of the same one are shown once.
- **A value the page left blank** (`<YOUR_API_KEY>`, `/path/to/project`) is asked for; a key is typed without showing.
- **Nothing found?** Some pages have no JSON, or keep it elsewhere. rak says so and asks you to paste it, or press Enter to answer three questions: a name, the command or web address, and any variables it needs.
- **In a chat,** `/mcp add` does the same, and the server's tools are usable at once, with no restart.
- **In a script,** `-y` skips the question when the source names one server with nothing to fill in; `rak mcp add -` reads the JSON from a pipe.

Or name it yourself, or edit the file:

```bash
rak mcp add files -- npx -y @modelcontextprotocol/server-filesystem .        # a program
rak mcp add docs --url https://example.com/mcp -H 'Authorization: Bearer KEY'  # a web address
rak mcp list                                                                  # try each one
rak mcp remove files
```

- **Where:** `~/.config/rak/mcp.json` for all projects, or `./.rak/mcp.json` with `--project`. Same shape as Claude Code and Cursor, so entries can be pasted across. `add` connects once so a typo shows straight away; `-e KEY=VALUE` sets an environment variable for a program. The file is readable by you only, since it can hold keys.
- **Names** are `server__tool`, so a server's `read_file` can't be mistaken for rak's.
- **Approval:** a tool the server marks read-only runs freely; any other asks first (`s` at the prompt allows all of that server's tools for the session). `--read-only` keeps only read-only tools; `--no-mcp` leaves servers out.
- **Large servers load when needed.** Tool descriptions are sent on every call, so a server whose descriptions pass about 1,000 tokens is named in one line instead, and its tools are sent only after the model calls `load_tools` for it.
- **A project's servers wait for trust.** A `./.rak/mcp.json` that came with a cloned repository is used only once you trust the folder (see [Safety](safety.md)).
- **Not there yet:** OAuth sign-in (a key in a header works), and MCP prompts and resources.
