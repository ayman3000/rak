# MCP servers

Tools from outside rak come through the [Model Context Protocol](https://modelcontextprotocol.io). A server is a separate program or a web address; its tools join rak's own. Back to the [README](../README.md).

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
