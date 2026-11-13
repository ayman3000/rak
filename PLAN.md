# rak: where things stand and what comes next

Updated 2026-11-13, after releasing v0.1.0-alpha.5. A working plan for picking the work up on another machine.

## Where things stand

- The newest release is **v0.1.0-alpha.5** (2026-11-13). It has everything
  below; `main` and the release match.

### In v0.1.0-alpha.5

- Falling back to another provider when the one in use fails (design below):
  `471c25d`, its messages made short and clear in `9d19ea3`.
- A warning when rak starts in the home folder or at the top of the disk
  (`60f1a20`): its tools can reach every file there. A warning only.

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
- [x] release.yml: upload-artifact v7, download-artifact v8 (Node 24), done
      2026-11-13; to be confirmed by the next release's build.
- [ ] Announce rak once the providers people will try first are confirmed.

## Porting what SwiftAgentKit has (gap list, 2026-11-13)

The owner's order: port what SwiftAgentKit already has before new ideas
(`/undo` is parked: SAK has no undo). Each SAK module was compared with
rak's source. Already ported: WriteVerifier (`tools/verify.rs`), ReadSlice,
the loop guard, FileStateRegistry (`Ctx::stale_reason`), skills
(`use_skill`, `save_skill`, import), saved chats, `delegate` (SAK's spawner
also runs one helper at a time by default), compaction, MCP.

In the order they are being built (value for a coding agent first):

- [x] 1. `apply_patch` (SAK `UnifiedDiff` + `PatchFileTool`): edits as a
      unified diff, several hunks in one approved step. ~300 lines.
- [x] 2. `fetch_url` with an SSRF guard (SAK `FetchURLTool`, `SSRFGuard`):
      a public page as readable text; private addresses refused. ~300 lines.
- [x] 3. Repair-retry (SAK `RepairRetryPolicy`): a tool failed and the model
      ends the turn claiming success: one "fix it or say it failed" nudge.
- [x] 4. Deferred tool groups (SAK `DeferredToolGroup`, `load_tools`): an
      MCP server's tools cost one prompt line until the model loads them.
- [x] 5. Stream stall limit (SAK `StreamStallPolicy`): a streamed answer that
      makes no progress for too long fails as retryable, instead of waiting
      out a 15-minute read timeout.
- [x] 6. Tool-output artifacts (SAK `ArtifactStore`, `artifact_read`): a
      trimmed tool output stays readable by id instead of being lost.
- [x] 7. Memory (SAK `RememberTool`, `AgentMemoryStore`): `remember` keeps a
      fact per project in a file, put in later system prompts.
- [x] 8. Routines (SAK `Routine`, `run_routine`): saved fixed steps run with
      no model call between them.
- [ ] 9. Record and replay (SAK `SwiftAgentKitReplay`): capture a live run's
      replies to a file and replay it in tests.

Lower value for rak, later or never: `PythonTool` (run_shell covers it),
Office/PDF text, `ToolTrace`, `StructuredOutput`, goals (`AgentGoal`; rak
has `todo`). Not for rak: SwiftAgentKitMac, SwiftAgentKitSimulator, Slack,
the `@Tool` macro.

## Design: falling back to another provider (built 2026-11-12, released in alpha.5)

Discussed with the owner on 2026-11-12; build only after his go-ahead. Hermes (~/.hermes/hermes-agent) was read for this: its api_max_retries, FailoverReason, restore_primary_runtime.

**Goal.** When the selected provider fails, rak carries on with another one
instead of stopping. No new settings: the chain comes from the key file.

**The chain.**
1. The selected provider and model (`RAK_PROVIDER`, `RAK_MODEL`, or what
   `--provider` / `-m` / `/provider` chose).
2. Then every other provider whose key line in `~/.config/rak/.env` is set,
   in the order the key lines appear in the file (local `ollama` and the
   other no-key servers last).
3. Each fallback uses the model last picked for that provider
   (`~/.config/rak/models`). A provider with no remembered model is skipped:
   rak never guesses a model name. A key in the file is not consent to spend;
   having used the provider in rak is.
4. Two entries that end at the same server (same address and key) count once.

**When to move on** (rak's errors are `Retryable` / `Fatal` today; this needs
a reason on each, like Hermes's `FailoverReason`):

| Failure | Retry the same model | Then |
|---|---|---|
| network, timeout, 5xx, overloaded | 2 times (today 5), backoff 1 s, 2 s | next provider |
| rate limit (429) | 2 times, honouring `Retry-After` up to 10 s | next provider |
| out of credit or quota (402, "insufficient balance") | no | next provider |
| bad key, model not found (401, 403, 404) | no | next provider, and say the config is wrong |
| context too long | no: compact (built) | next provider only if compaction cannot fix it |
| a request rak built wrongly (400), a refusal | no | stop, as today: another model will not fix it |

**Consent and visibility.**
- In a chat, ask once per switch: "ollama-cloud failed 3 times (503).
  Continue with anthropic · claude-sonnet-5-5? [y/n]". No stops the request.
- With `-y`, or with no one to ask (scripts, CI): switch without asking.
- Every switch is printed, logged (`--log`), and in `--json` and `/usage`
  (which model answered which turn).

**Switching.**
- Never mid-answer: if text was already shown, that turn is redone on the
  next model, under a note that says so.
- The conversation carries over, as with `/provider`. The context window is
  re-read for the new model; compaction runs if it is smaller. Anthropic's
  prompt cache is lost on a switch.
- Turn-scoped, as in Hermes: the next request starts on the selected provider
  again. Exception (also from Hermes): if the selected one failed for good
  (bad key, model not found), the chat stays on the fallback and says so,
  rather than failing again on every request.
- The `delegate` helper uses the same chain.
- Chain exhausted: stop with every attempt listed (provider, model, error).

**As built.** `src/fallback.rs` (failure kinds, the hook), `Agent::fallback`
and `Agent::switches`, `src/cli/chain.rs` (the chain, the question). Differences
from the design above: the retry wait is a fixed 1 s then 2 s (`Retry-After`
is not read); no duplicate check, since no two providers share an address; an
overflow that compaction can't fix falls back too. Tried with stand-in servers
(busy, bad key, yes, no, back on the chosen one, staying after a bad key), then
with real providers on 2026-11-13: ollama-cloud with a model it doesn't have
(404) fell back to anthropic · claude-haiku-5-5, in a chat (y, n) and in a
one-off run; the owner saw the same in his own terminal. A busy or
rate-limited real provider has not been seen yet.

**Not now** (Hermes has them; add only if needed): waiting out a rate limit
that lifts within N seconds instead of switching; repeated wait-and-retry
cycles after the whole chain fails.

**Tests.** A pure `chain()` from the key file and the remembered models
(order, skips, duplicates); the failure table as a pure `next_step(error)`;
stand-in servers for "first fails, second answers", "fails mid-answer",
"all fail", "permanent failure stays on the fallback"; the chat question and
`-y` driven through `script`.

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
