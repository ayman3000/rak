# rak: where things stand and what comes next

Updated 2026-11-12, after releasing v0.1.0-alpha.4. A working plan for picking the work up on another machine.

## Where things stand

- The newest release is **v0.1.0-alpha.4** (2026-11-12). It has everything
  below; `main` and the release match.

### In v0.1.0-alpha.4

- Large projects: saved chats (`rak -c`, `/resume`), `todo`, `delegate`,
  `code_map`, `read_file` line ranges, stubs that name their call.
- Compaction (`src/compact.rs`): when trimming is not enough, the older part
  becomes a model-written summary; a "too long" error summarizes and retries.
- Named providers (`src/providers.json`): 15, each with its own address and
  key; `--provider`, `RAK_PROVIDER`, `/provider` with an arrow-key list,
  the last model remembered per provider, `rak keys` listing every provider,
  `rak keys --update` for older key files, the last value set wins.
- Fixes from rak's own review: trim underflow, `--context` overflow, the
  turn-limit summary error, background commands reaped and their logs
  removed, `vite build` kept in the foreground, MCP servers stopped on every
  way out (usage error, `rak mcp add/list`, Ctrl-C), parsers stopped with
  their children.
- Tried against real servers: `ollama`, `ollama-cloud`, `anthropic` (README,
  "Tested so far"). The other providers have not had a real request yet.

## Next steps

### 1. Test what has not been tried

- [ ] `code_map` on a codebase with thousands of files (for example a clone of
      a large open-source repo): check the 14,000-char and 5,000-file limits,
      the "map a folder with `path`" note, and how long it takes. No model
      needed.
- [ ] A session that fills a full-size context, in a container: check that
      trimming and then compaction keep the `todo` list, that the model makes
      sense of the stubs and the summary, and what the summary call costs.
- [ ] Anthropic's real "prompt is too long" error triggering the retry (only
      a stand-in has so far).
- [ ] `delegate` with Claude (only kimi has used it so far).
- [ ] The model choosing `delegate` without being asked. kimi-k2.6 never
      did; deepseek-v4.1-flash did on its own (2026-11-12, "check the
      codebase": about 38 reads in the helper, 1.3M input tokens for the
      request, 93% cached). So it depends on the model. Still to do: try
      more models, and cap what a helper may spend (its 25 turns re-send a
      growing context).
- [ ] Fix whatever these turn up, with a unit test for each fix.

### 2. Try the untested providers

- [ ] One real request each for openai, gemini, zai, zai-coding, openrouter,
      huggingface, groq, deepseek, mistral, lmstudio, llamacpp, vllm; update
      the README's "Tested so far" table.
- [ ] Ollama's cloud lists models without `:cloud` (`kimi-k2.6`); check
      whether `kimi-k2.6:cloud` works there or needs the suffix dropped.

### 3. Larger gaps after that

- [ ] Undo for the agent's changes (`/undo`), git-aware where the project is a
      git repo.
- [ ] Several edits in one approved step.
- [ ] From README "What is missing": Ctrl-R history search, the "still
      running" note during a long tool call, fetching a whole web page,
      Gemini's own API.
- [ ] Esc stops a running request, as Ctrl-C does (Esc already cancels the
      /provider and /model lists).
- [ ] RAK_PROVIDER and RAK_MODEL from different files (a project's .env and
      the global one) can still pair a model with the wrong provider: take
      the model from the same place as the provider.
- [ ] release.yml: actions/upload-artifact and download-artifact are still
      v4 (Node 20, deprecated); move to v7 / v8 and check with a release.
- [ ] Announce rak once the providers people will try first are confirmed.

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
