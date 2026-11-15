# Safety

What rak does before it changes anything, and where its limits are. Back to the [README](../README.md).

- **Edits and commands ask first**, every time, unless you answer "always" or start with `-y`. The prompt shows the whole command, every line of it.
- **A no stops the request.** The model doesn't get another turn to try the same change a different way.
- **No terminal, no changes.** In a script or CI without `-y`, rak stops with exit code 3 instead of guessing.
- **A tool error is not passed off as done.** When the step before the answer had a tool error (an edit that didn't apply), rak asks the model once to fix it or say plainly what failed; at most three times a request.
- **A request that edited files must run the project's gate**, if it has one (see [Project notes](memory-and-skills.md#project-notes-the-rak-folder)).
- **Every write is checked first** for leaked patch text and broken syntax (see [Tools](tools.md#every-write-is-checked-first)).
- **File tools stay in the project folder** unless you pass `--any-path`. Symlinks are resolved first.
- **`run_shell` is not confined.** A command you approve can do anything your user can. Read it before you say yes.
- **Starting in your home folder or `/`** shows a warning first, since the agent could then reach everything below it.
- **A folder's own settings wait for your trust** (from maki). A cloned repository can ship a `.rak/mcp.json` (its servers are programs rak would start), a `.env` that points rak's requests, and your key with them, at another address (`RAK_BASE_URL`), and skills or routines for the agent to follow. rak uses none of that until you say you trust the folder: it asks once, in a terminal, and remembers the answer in `~/.config/rak/trusted`. Without a terminal (a script, CI, an unattended `-y` run) an untrusted folder's settings are left out and rak says so; `--trust` uses them for one run. `rak mcp add --project` and `rak skills add --project` trust the folder, since you put them there. The project's instructions (`AGENTS.md`, `rak/rules.md`) are read as before.
- **Keys stay put.** The key file is created readable by you only, a key goes only to its own provider's server, and values read from key files are not passed on to the commands the agent runs (see [Providers](providers.md#keys-and-settings)).
- **`fetch_url` refuses** addresses on this machine or a private network, even through a redirect.
- **An MCP server is a program you chose to run.** rak starts it with your rights and trusts what it says about which tools are read-only.
