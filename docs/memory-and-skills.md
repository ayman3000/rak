# Project notes and skills

How rak remembers a project between sessions, and how it learns know-how it can load when a task needs it. Back to the [README](../README.md).

**Contents:** [Project notes: the `rak/` folder](#project-notes-the-rak-folder) · [Skills](#skills)

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

- **The first chat offers them.** In a project with no notes, rak says so and asks once whether to set them up (Enter for yes; `n` is remembered for that folder).
- **Or start with `/init`.** The agent looks at the project (map, README, build and test files, git log) and writes `project.md` and `roadmap.md`, each write approved. Run it again later to bring them up to date. rak never creates the folder on its own.
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

## Skills

A skill is know-how in Markdown: how to build an iOS app from the terminal, how this team deploys, how a library is meant to be used. The prompt lists each skill as one line, and the agent loads the full instructions with `use_skill` when a task matches.

- **Where:** `.rak/skills/` in the project, then `~/.config/rak/skills/`; a project's skill wins over one with the same name.
- **Two formats:** `NAME.md` as in SwiftAgentKit and sak (`# Name`, `Description:`, then the instructions), or a folder with a `SKILL.md` as in Claude's Agent Skills. Files a skill ships go beside it.
- **Built in:** `iOS app` (macOS only: builds an app with `swiftc` and runs it in the Simulator) and `Write a skill`. A skill of yours with the same name replaces a built-in one; `--no-skills` leaves all out.
- **The agent writes skills too.** Ask for one and it interviews you, shows the draft and saves it with `save_skill` once you agree. After you correct it, it may keep the lesson as a skill. Every save asks first and shows the whole skill.

**Adding skills from elsewhere:**

```bash
rak skills add https://github.com/anthropics/skills/tree/main/skills/pdf
rak skills add https://github.com/anthropics/skills        # every skill in a repository, one at a time
rak skills add ./deploy.md --project
rak skills                    # list them
rak skills remove pdf
```

**Each skill is a command.** In a chat, `/sdd-plan` (the skill's name, with dashes for spaces) asks the agent to load that skill and follow it; what you type after it is the request. The agent still loads a skill by itself when a task matches its description; the command is for when you want one for sure, or for step-by-step sets where you choose the step. A skill named like one of rak's own commands gets no command and is loaded by asking for it.

In a chat, `/skills add URL` and `/skills remove NAME` do the same. `/skills remove` asks before deleting, and finds the skill whether it is this project's or yours; a skill built into rak can't be removed.

`rak skills add` takes a web address (a GitHub file or skill folder, with the files it ships) or a file. A GitHub repository's own page, or a folder holding several skill folders, stands for every skill in it: rak lists them, then shows and asks about each in turn. Before saving it shows the skill and points out lines worth reading first: commands piped into a shell, deletions, `~/.ssh`, `.env`, text telling the agent to ignore you. That list is a hint, not a verdict: read the skill.
