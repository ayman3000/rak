# rak: where things stand and what comes next

Updated 2026-11-12. A working plan for picking the work up on another machine.

## Where things stand

- `main` holds the large-project work (`06cbd54`, CI re-run as `49e7d9a`): CI is
  green on Linux, macOS and the oldest supported Rust.
- The newest release is **v0.1.0-alpha.3**. It has the skills work (writing
  skills, keeping lessons, `rak skills add URL`) but **not** the large-project
  work below.
- The CI run for `06cbd54` (43097912846) is stuck "queued" on GitHub's side and
  can't be cancelled; ignore it.

### In main, not released yet

- Saved chats, one file per chat per project in `~/.config/rak/sessions/`;
  `rak -c` continues the last, `/resume` lists and picks one (`src/sessions.rs`).
- `todo`: the agent's task list; if it finishes with steps open, rak sends it
  back once (`src/tools/todo.rs`, `Agent::finish_check`).
- `delegate`: a helper agent with read-only tools and a fresh context, on the
  current model (`src/delegate.rs`).
- `code_map`: source files with their definitions and lines; `read_file` takes
  `start_line` / `end_line` (`src/tools/map.rs`, `src/tools/fs.rs`).
- Trimmed outputs say which call they came from; the task list is never trimmed.

## Next steps

### 1. Test what has not been tried

- [ ] `code_map` on a codebase with thousands of files (for example a clone of
      a large open-source repo): check the 14,000-char and 5,000-file limits,
      the "map a folder with `path`" note, and how long it takes. No model
      needed.
- [ ] A session that fills the context, in a container: check that trimming
      keeps the `todo` list and that the model makes sense of the stubs.
- [ ] `delegate` with Claude (only kimi has used it so far).
- [ ] The model choosing `delegate` without being asked. It doesn't yet;
      try rewording `BIG_PROJECTS` in `src/cli/session.rs` or the tool's
      description, and measure the token cost of any change.
- [ ] Fix whatever these turn up, with a unit test for each fix.

### 2. Publish v0.1.0-alpha.4

- [ ] Bump the version in `Cargo.toml`, `README.md` (status line and
      `RAK_VERSION` example) and `install.sh`.
- [ ] `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- [ ] Commit, push, wait for CI to pass.
- [ ] `git tag v0.1.0-alpha.4 && git push origin v0.1.0-alpha.4`; the release
      workflow builds the four archives and publishes them.
- [ ] Check the install script picks up the new release.

### 3. Larger gaps after that

- [ ] Undo for the agent's changes (`/undo`), git-aware where the project is a
      git repo.
- [ ] Several edits in one approved step.
- [ ] From README "What is missing": Ctrl-R history search, the "still
      running" note during a long tool call, fetching a whole web page,
      Gemini's own API.

## Working rules

- SwiftAgentKit is the design reference; read it, never change it (nor sak).
- API keys live in `~/.config/rak/.env`. Never print, read or copy that file.
- Agent runs with `-y` (auto-approve) only inside a container, never on the
  host. On the host, approve each step after reading it; no "always".
- Push, tag and publish only after an explicit go-ahead.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Setting up on another machine

```bash
git clone https://github.com/ayman3000/rak && cd rak
cargo test
cargo build --release   # target/release/rak
```

Put keys in `~/.config/rak/.env` on that machine (see README, "Use").
