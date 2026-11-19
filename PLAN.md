# rak: where things stand and what comes next

Updated 2026-11-18, after releasing v0.1.0-alpha.37. A working plan for picking the work up on another machine.

## Where things stand

- The newest release is **v0.1.0-alpha.37** (2026-11-18). It has everything
  below except alpha.25's prompt rule, which alpha.26 took out; `main` and the
  release match. Benchmark-run notes are kept outside the repo.

### On main, not released

Nothing yet.

### In v0.1.0-alpha.38

Five findings from a review of alpha.37 on the laptop, one dead end met on
another machine, and helpers following `--tools`.

- **Helpers follow `--tools`** (session::tool_allowed, Helpers::make): a
  helper's tools are cut by the same list as the run's. Before, a helper
  kept fetch_url whatever the list said. Proven against a stand-in server:
  with fetch_url left out, the helper's request carried list_dir,
  search_files, code_map, read_file; with it in, fetch_url too.
- **Paths where keys live** (tools::never_reason): block_reason told the
  model to ask for /add-dir for every outside path, and /add-dir refuses
  ~/.config/rak, ~/.ssh, ~/.gnupg, ~/.aws, ~/.kube and ~/.docker. Now those
  get their own message (never opened, do not ask to add it), and
  ~/.config/rak also names past_chats. Seen on a Linux machine: a model
  asked twice for `/add-dir ~/.config/rak` to read the last chat.
- **A folder pick of ChatGPT while signed out** (start_at_pick) falls back
  to the default with a note, as a missing key does.
- **GitHub's `truncated` flag** (fetch::tree, cut_short, placed_under): a
  repository too large to list in one request is an error that asks for the
  skill's folder; a skill folder in one is listed by its own tree
  (`git/trees/BRANCH:FOLDER`, checked against anthropics/skills).
- **One model-list fetch per ChatGPT model object** (models_once): the
  context window and "sees images" each fetched it before.
- **Sign-in callback** (auth::answers): a request with another state, or
  none, gets a 400 and the wait goes on; a refusal with no state is still
  reported. The browser page no longer says "signed in" before the code is
  exchanged.
- README: the Features table said "about 4 MB of memory", an alpha.1
  figure; now 18 MB on a Mac (alpha.33) and the binary as under 5 MB.
- The leaderboard command was run whole against the stand-in server: -y,
  --json, --log, --base-url, -m, --context, --max-tokens, --time-limit and
  --tools are all accepted, RAK_API_KEY is sent as the bearer token,
  max_tokens arrives as given, and no effort is sent.

### In v0.1.0-alpha.37

- **Every skill is a chat command**: `/sdd-plan add login` sends "Use the
  skill "sdd-plan": load it with use_skill and follow it for this request:
  …" (menu::skill_request). The command is the skill's name as a slug; the
  "/" menu lists skills after rak's own commands ("skill: " + description),
  /help names them, and a typed prefix resolves to one. A slug that is one
  of rak's own commands, or another skill's, gets no command. The list is
  rebuilt before every prompt, so /skills add and remove show at once.
  Driven in a real chat through a pty (gpt-5.6-luna): the menu, /help and
  /sdd-plan, which loaded the skill and stopped at its missing inputs.

### In v0.1.0-alpha.36

Checked against GitHub after the release, once its limit had reset: an
8-skill repository cost 2 API requests and saved 8 of 8, and a skill folder
that ships files (anthropics/skills pdf) came with its 11 files.

- `rak skills add … -y` no longer prints each skill's first 40 lines: with
  nothing asked, a skill is its name, description, source and length, the
  files it ships and any flagged lines. Without -y nothing changed.
- Adding a repository's skills costs GitHub's API two requests (the
  repository, then its file list, kept for the run), not one more per skill
  folder: the files a skill ships are read from that list and downloaded
  raw. GitHub allows 60 requests an hour unsigned, and a few runs on an
  8-skill repository had used them up. At the limit rak now says so, with
  the minutes until it opens again.

### In v0.1.0-alpha.35

- `rak skills add` (and `/skills add`) with a GitHub repository's page, or a
  folder of skill folders: every folder with a SKILL.md is found through
  GitHub's tree API (fetch::expand), listed, then shown and asked about one
  at a time; one that can't be read is skipped. Before, a repository's page
  was fetched as a skill file and refused as "over 256 KB". Tried on
  sivaprasadreddy/sdd-skills (8 skills).
- `/skills remove NAME` in a chat (the command line had `rak skills remove`
  already): asks first, deletes the skill where it lives (this project's or
  the person's), and takes it out of the run's list; a built-in skill it
  stood in for comes back, and a built-in one itself is refused.
- A SKILL.md description written as YAML block text (`description: >` and
  the lines under it) is read whole; it used to be read as ">".

### In v0.1.0-alpha.34

- **The ChatGPT plan route now uses the prompt cache.** Every request of a
  conversation carries one random `session_id` header (and the same
  `prompt_cache_key`). Without the header the route cached nothing of a
  repeated 4K-token prompt (0 of 4,238, four times); with it, 4,096 from
  the second request on (the key alone did nothing). Same two read-only
  tasks on rak's code, gpt-5.6-luna, 2026-11-18: cached 24% → 90% (138K of
  580K → 392K of 435K) and 1% → 84% (3K of 352K → 789K of 935K); the runs
  took different paths, so only the shares compare. The header is not in
  OpenAI's docs for this route. gpt-6.2-sol hit less steadily in the direct
  test (one of two repeats).
- Memory, alpha.33 on macOS, read-only tasks, MCP off: 18.0 MB resident
  for a one-word reply, 19.0 to 20.7 MB on 350K to 580K-token tasks
  (footprint 8.6 to 11.3 MB). The 4.0 MB in the README's benchmark table is
  alpha.1 and not comparable.

### In v0.1.0-alpha.33

- The README has a "What's new" section, kept current with every release,
  and its Features section lists everything by purpose.

- **Sign in with ChatGPT** (the `chatgpt` provider; `rak login chatgpt`, `rak login chatgpt
  --new`, `rak login chatgpt --status`, `rak logout chatgpt`): requests on the person's
  ChatGPT plan, no API key. Built on OpenAI's documented open-source route
  (developers.openai.com/siwc/token-sharing-open-source): rak registers as
  itself (`dynamic_agent_client`, name "rak") and keeps the client id it is
  issued; PKCE, callback on 127.0.0.1:1455 (or a free port), ID token
  checked against OpenAI's keys (ring), tokens in ~/.config/rak/chatgpt.json
  (0600), renewed under a file lock. Requests: POST /v1/responses, store
  false, stream true, system prompt as `instructions`, the reply's own items
  (reasoning with encrypted_content) sent back unchanged. Also in setup,
  /provider (signs in on the spot), `rak keys` and the fallback chain.
  **Tried for real 2026-11-18** (gpt-5.6-luna and gpt-6.2-sol): sign-in and registration, the model list (12 models, the
  shape the docs give, with context_window and input_modalities), a plain
  request, tool calls with plain function tools (no "namespaces" needed),
  `include: reasoning.encrypted_content`, an 8-turn run with the reasoning
  items sent back, and a forced token renewal. Efforts low to max go as they
  are. Not measured: how much of the prompt the plan route caches. One
  sign-in failed in the browser with OpenAI's own
  `login_challenge_not_found_in_session` (their page lost its session); the
  retry worked with no change. Claude Pro/Max
  sign-in is not built: Anthropic allows it only in its own apps.
- `/add-dir PATH` in a chat and `--add-dir PATH` at start: the file tools
  reach that folder too, for that run (Ctx::add_folder). Only the person
  adds one. Shown as resolved; refused: the disk's root, a file, a folder
  inside the project, and any folder that is, holds or is inside a place
  keys live (~/.ssh, ~/.gnupg, ~/.aws, ~/.kube, ~/.docker, rak's config),
  so the home folder itself. `/add-dir` alone lists what is reachable.
- The "outside the project" refusal now tells the model to stop and name
  /add-dir in a line or two, and not to work around it (a blocked build
  had cost 459,070 input tokens ending at "blocked").

### In v0.1.0-alpha.32

- Saving a chat no longer opens every other chat: the clean-up past 30 goes
  by each file's write time (sessions::past_keep), not by parsing them all.
  Measured with 30 chats, median of 9: saving one went 3.4 → 0.5 ms at
  100 KB a chat, 28.0 → 2.3 ms at 1 MB, 149.0 → 15.7 ms at 5 MB.
  Listing is unchanged (3.0 / 25.4 / 136.0 ms): it still parses each chat.

### In v0.1.0-alpha.31

- `Thinking… 12s` while a model call is out and nothing is arriving: one dim
  line, drawn after the first second, redrawn each second, erased when the
  answer or a tool line is shown; a wait of 5 s or more leaves `Thought for
  Ns`. One sleeping thread (cli/thinking.rs), started on first use; the agent
  says when a call begins and ends (Event::Calling / Replied). Terminal only.
  Measured against alpha.30 on an instant fake server, 40 one-off runs each:
  150.7 → 150.8 ms without a terminal, 153.2 → 153.1 ms in one. No reasoning
  text or token count is shown (usage arrives only at the end of a call).
- `past_chats` (tools/chats.rs): the model can list, read and search this
  project's earlier saved chats; requests and answers only, tool output left
  out, 12,000 characters a call with `from` to go on, the current chat left
  out (sessions::current). Chats only: measured 574 bytes of tool definition
  (all tools 10,833 → 11,409 bytes); a one-off run is unchanged (16 tools,
  10,833 bytes). Chats past the newest 30 are deleted, so a project's very
  first chat may be gone.
- Answers wrap at words (markdown::wrap), list items continue under their
  text, code is never re-wrapped; a word wider than the terminal stays whole.

### In v0.1.0-alpha.30

- A folder remembers its pick: `/provider`, `/model` and `/effort` write
  `.rak/model` in the project folder (provider, model, effort: names only,
  no other line is read), and the next chat there starts with them. Flags
  and RAK_PROVIDER/RAK_BASE_URL in the environment win; a one-off run does
  not read it; a provider whose key is gone is said and skipped. Read and
  written only in a trusted folder; the file does not itself ask for trust.
- `rak keys set --help` prints its own help (was the `rak keys` text).

### In v0.1.0-alpha.29

- Keys without opening the file: `rak keys set NAME` (a provider, a key's
  own name, or tavily; no name: a list), `/key` in a chat, and `/provider`
  asks for the key when the provider picked has none. Pasted without being
  shown, checked where the provider has a free check, saved 0600 on its own
  line; a replaced key keeps the old file as .env.bak. Never an argument,
  never printed, never in a chat, saved chat or log. A key given in a chat
  is used at once (settings::give_key).
- The "/" menu shows up to 20 rows (was 16): every command at once.

### In v0.1.0-alpha.28

- Reasoning effort as a setting: `RAK_EFFORT` in the key file (a bad value
  is named and ignored), `--effort` still wins for one run, and `/effort`
  in a chat (picker, or `/effort high` / `default`) changes it for the chat
  and its helpers. The status line shows the level being sent.
- A chat starts at medium effort when none is set; a one-off run sends none
  (benchmarks keep the model's own default). A server that refuses the chat's
  default (400/422) gets the request again without it, and that model gets
  none for the rest of the run; an effort the person set is never dropped.
  `default` sends none on purpose.

### In v0.1.0-alpha.27

- A plain `rak update` only installs a release newer than the running rak;
  a build newer than every release is left alone and says so. A version
  named on the command line is still installed, older or not.
- `rak update` passes the installer to `sh -s` on stdin; it is no longer
  written to a predictable file in the temp folder.

### In v0.1.0-alpha.26

- `rak update` installs the newest release in place (`rak update VERSION`
  for one release, `--check` to only look), through that release's own
  install.sh. install.sh says "Went back" when it installs an older version.
- alpha.25's stated-number rule taken out of the system prompt.
- The Harbor adapter, the probes and the benchmark notes moved out of the
  repository.

### In v0.1.0-alpha.25

- The system prompt: numbers the request states are checked against the
  output; a measured target is met with room to spare and measured from the
  saved result in a fresh process; an exact value is matched exactly;
  something unexpected is looked into, not explained away. Reverted on
  `main` (2026-11-17): about 58% more cost for no clear gain.

### In v0.1.0-alpha.24

- /jobs lists the servers run_shell started in the background (state,
  minutes, pid, command, log); /jobs stop N|all stops one with its process
  group (SIGTERM, then SIGKILL after 2 s). A chat that ends with some still
  running asks whether to stop them; a one-off run leaves them running.

### In v0.1.0-alpha.23

- MCP images: a tool's image (chrome-devtools take_screenshot) was dropped
  as "[image not shown]", and the model claimed it showed it. Now it is saved
  to <temp>/rak-images/<tool>-<time>.png, the result names the file and its
  size, and the first image goes to the model like view_image's: shown,
  described by the vision model, or left out with a note (Agent::take_image,
  for any tool's image).
- Short LaTeX formulas of known symbols in answers are drawn as characters
  ($\rightarrow$ as →); prices and unknown commands stay as written.

### In v0.1.0-alpha.22

- A pasted image shows as `[image N]` in the line (its path goes with the
  request), not the long temp path.
- Whether a model sees, for servers that don't say (not Ollama): known
  families see (Gemini, GPT-4o/4.1/5, o1/o3/o4, Claude, Gemma 3/4, Llama 4,
  VL/vision/llava/pixtral); DeepSeek's text models don't; others unknown.
  So a Gemini or GPT model gets images itself, not through the vision model.
- First real use (2026-11-16): Ctrl+V pasted a screenshot and
  gemma4:31b-cloud (sees) read it directly. A vision model on ollama-cloud
  needs the name without `-cloud` (gemma4:31b); `-cloud` is the local Ollama
  app's name for forwarded models.

### In v0.1.0-alpha.21

- A vision model for a main model that can't see (Naseem's "describe" mode):
  RAK_VISION_PROVIDER / RAK_VISION_MODEL in the .env (or --vision-provider,
  --vision-model). Images pasted with Ctrl+V (Alt+V too; osascript, wl-paste
  or xclip, PowerShell) or dragged in as paths go to it with the message, and
  its description goes into the message for the main model; view_image
  (with an optional `question`) routes the same way. A model that sees gets
  the images itself. With none set and a model that can't see, a hint and the
  message as typed. Bare paths attach only with someone at a terminal, so an
  unattended task naming an image is unchanged. The welcome panel has a
  `vision` line; `rak keys` and the template know the two settings. Tested
  with unit tests and a scripted vision model; not yet with a real one, and
  Ctrl+V with an image on the clipboard not tried (the script runs and says
  when there is none).
- From a user run (gemma4:31b, alpha.13) extracting the Rust book's code: the
  chapter list loads from `toc-*.js` (and a `toc.html` frame), so fetch_url
  and curl saw one chapter link; the model curled the index ~20 times, ran
  out of turns, then guessed the list and reported empty files as done.
  - fetch_url now names what it dropped but a browser loads: "Frames on this
    page: toc.html" and "Scripts on this page (not run; ...): toc-....js, ...",
    analytics left out, 8 of each at most.
  - The 5th and later fetch of one public URL in a request (fetch_url, or
    curl/wget in run_shell; fragment, trailing / and index.html ignored;
    localhost left out) gets a note to change approach: the loop check only
    caught identical calls with identical results.

### In v0.1.0-alpha.20

- The review before the answer is opt-in (`--review`); `--no-review` is
  still accepted.

### In v0.1.0-alpha.19

- Review before the answer: a one-off run (not a chat) that changed
  something is sent back once, at its first final answer, to list each
  requirement, rule and symptom in the request and its docs, quote it, name
  the check it ran, and fix what has none or fails; docs win over code
  comments. rak sends it, so it does not depend on the model following a
  prompt rule. Skipped with under 15% of `--time-limit` left; `--no-review`
  turns it off. About 1-3 extra model calls.
### In v0.1.0-alpha.18

- Contract and symptom rule in the system prompt: code others call keeps the
  names, arguments and layout the request, docs or structure imply (don't
  change a signature because another scored better in your own check); each
  described symptom or rule gets a check built from the request's own words;
  docs win over code.
### In v0.1.0-alpha.17

- Plan first: for a task with several steps the model starts with a short
  plan in `todo`, each step a checkable requirement of the request, with
  short thinking; todo updates go in the same reply as other tool calls.
### In v0.1.0-alpha.16

- `--time-limit SECS`: the run's time, counted from start. The task tells
  the model how long it has, and at 70% and 90% it is told what is left and
  to finish the essentials and save its outputs (`time_note` in agent.rs,
  added after the tool results, so the cache holds).
- The system prompt asks the model to re-read the request and check each
  thing it asks for before answering, and to batch independent tool calls and
  related shell commands.
- install.sh retries wget downloads.

### In v0.1.0-alpha.15

- Trusts the system's certificates as well as the built-in list
  (`src/http.rs`, used by every HTTP client). Behind a proxy that presents
  its own CA, every model request had failed with "invalid peer
  certificate: UnknownIssuer". Checked with a
  throwaway CA on macOS (`SSL_CERT_FILE`) and in Debian (system store), and
  a container with no certificates still reaches HTTPS sites.

### In v0.1.0-alpha.14

- Key files are owner-only from the moment they exist: the key file, its
  backups and the MCP server list are created with mode 0600 and renamed
  into place (`src/private_file.rs`); before, they were written first and
  restricted after.
- README rewritten as a landing page (banner, badges, "Why rak", grouped
  features); the detailed reference moved into guides under `docs/`.
- Releases are normal ones, marked latest, alphas included (alpha.13 was
  switched by hand); this is the first tag published that way.

### In v0.1.0-alpha.13

- Knows when it is done: the standing instructions now say to stop once what
  was asked works and is checked, and to name extra work in the summary
  instead of doing it.
- Setup checks a key where the model list would not: OpenRouter
  (`/api/v1/key`) and Hugging Face (`whoami-v2`), set as `key_check` in
  providers.json. A refusal asks "Type it again?"; "n" goes on, so a wrong
  check address can never lock out a good key. Gemini's 400 counts as a
  refused key. ollama.com has no check: its `/api/ps` refuses a good key too
  (found with a real key), and its model list is public.
- Not verified with a real key: the OpenRouter and Hugging Face checks
  (both refuse a fake key with 401).

### In v0.1.0-alpha.12

- First-run setup: a chat with no key file and nothing chosen asks for a
  provider (local servers that are running first), its key (hidden, saved
  0600) and a model from the provider's list, then starts. `rak setup` runs
  it again and keeps the rest of the file. A refused key is said plainly;
  scripts never get questions.
- install.sh says "Upgraded A → B" on an upgrade, and only "Start a chat
  with: rak" when a key file exists (checked, never read).

### In v0.1.0-alpha.11

- Folder trust (from maki): a folder's own `.rak/mcp.json`, `.env` rak
  settings, skills and routines are used only once the folder is trusted
  (asked once in a terminal, remembered in ~/.config/rak/trusted; `--trust`
  for one run).
- `run_shell` keeps the start and the end of long output, so the error at
  the end of a failing build reaches the model.

### In v0.1.0-alpha.10

- One request has no turn limit by default; an unattended run (one request
  with -y) keeps nothing; `reasoning_content` goes back within a request.
- `--tools LIST` / `--tools none`; the welcome panel follows it.
- Helper spending cap (`--helper-budget`, 200K input tokens); an empty
  helper answer comes back as an error.
- Markdown tables drawn as tables in the terminal.

### In v0.1.0-alpha.9

- Parallel helpers: several `delegate` calls in one reply run at the same
  time (3 on commercial cloud APIs, 2 on Ollama's cloud, 1 locally), steps
  tagged `[1]`, `[2]`…, fallback per helper (SAK `SubAgentGate`).
- `delegate`'s description names the helper's tools; the footer, `/usage`
  and `--json` count the helpers' tool calls apart.
- README rewritten; benchmarks and field notes moved to `docs/`.
- Still open: Markdown tables show as raw pipes in the terminal.

### In v0.1.0-alpha.8

- `/init`: the agent writes rak/project.md and rak/roadmap.md from what is in
  the project, or brings them up to date; the no-notes prompt line suggests
  it; the notes update is told which files a request changed.
- Still open: Markdown tables show as raw pipes in the terminal.

### In v0.1.0-alpha.7

- Project notes in a visible `rak/` folder, from Naseem (section below):
  project.md, roadmap.md, journal.md, rules.md with `## Gate`, memory.md;
  loaded at the start, journal and roadmap updated after a request that
  changed something; `--no-notes`.
- Seen in a real run (owner, Opus 5.5, a SQLite explorer): Markdown tables
  in answers show as raw pipes — the terminal renderer has no tables yet.

### In v0.1.0-alpha.6

- The SwiftAgentKit ports (gap list below): apply_patch, fetch_url, the
  repair nudge, deferred MCP servers, the stall limit, kept tool outputs
  (artifact_read/search), remember, routines, --record/--replay.
- release.yml's artifact actions on v7/v8 (Node 24); this release's build
  is their first real run.
- Cost per request, measured on ollama-cloud with a no-tool prompt: 2,211
  input tokens against alpha.5's 1,824 (+387). The on-demand tools
  (load_tools, artifact_*, run_routine) cost nothing until needed.
- Not tried with a real model: the repair nudge; the stall limit against a
  real stuck provider (a local stand-in only).

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
- [x] `delegate` with Claude: Haiku 5.5 chose two parallel helpers on its own
      (2026-11-14), both answers right.
- [ ] The model choosing `delegate` without being asked. kimi-k2.6 never
      did; deepseek-v4.1-flash did on its own (2026-11-12, "check the
      codebase": about 38 reads in the helper, 1.3M input tokens for the
      request, 93% cached). So it depends on the model. Still to do: try
      more models. Done: a helper's spending cap (`--helper-budget`, 200K
      input tokens by default; warned at three quarters, made to answer at
      the cap), built 2026-11-14 and checked live on kimi-k2.6:cloud.
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
- [x] Several edits in one approved step (`apply_patch`).
- [ ] From README "What is missing": Ctrl-R history search, the "still
      running" note during a long tool call, Gemini's own API. (Fetching a
      page is done: `fetch_url`.)
- [x] Markdown tables drawn as tables in the terminal: aligned, fit to the
      width, long cells wrapped (2026-11-14).
- [ ] Esc stops a running request, as Ctrl-C does (Esc already cancels the
      /provider and /model lists).
- [ ] RAK_PROVIDER and RAK_MODEL from different files (a project's .env and
      the global one) can still pair a model with the wrong provider: take
      the model from the same place as the provider.
- [x] release.yml: upload-artifact v7, download-artifact v8 (Node 24), done
      2026-11-13; confirmed by the alpha.9 release build.
- [ ] Announce rak once the providers people will try first are confirmed.

### 4. A mic: speaking a request (planned 2026-11-18, not started)

The aim: say a request instead of typing it, in a chat. Asked for on
2026-11-18; set down here for later. Nothing is built.

**What works today with no code.** macOS dictation types into rak's prompt
like a keyboard: the mic key, or Fn twice, in the terminal. Arabic and
English. Try this first: if it is enough, rak needs no mic of its own, and
the README can say so in one line.

**Why it is not a small feature.** Two separate parts, neither in rak now:

1. *Recording.* A terminal program does not get the mic for free.
   - Bundled audio code (the cpal crate: CoreAudio on macOS, ALSA on
     Linux) makes the binary bigger and breaks the static musl Linux
     builds, which have no ALSA to link. Not this way.
   - Calling a recorder that is already installed: `ffmpeg` (macOS:
     `-f avfoundation -i ":0"`; Linux: `-f pulse -i default` or `-f alsa`),
     or `sox`/`rec`, or `arecord` on Linux. No new dependency in rak; said
     plainly when none is found. This way.
   - macOS asks once for microphone permission, for the terminal app, not
     for rak.
2. *Speech to text.* Needs a speech model.
   - A provider with a transcription endpoint and its own key: Groq
     (whisper-large-v3, OpenAI-shaped `POST /audio/transcriptions`) and
     OpenAI (`OPENAI_API_KEY`). One multipart upload; ureq has no
     multipart helper, so the body is built by hand (small).
   - Not the ChatGPT plan: OpenAI's Sign in with ChatGPT docs say the
     transcription API is not supported on that route.
   - A local model (whisper.cpp) is a download of hundreds of MB and a
     program to install: only as "a command of your own", below.

**The design to build, lightest first.**

- `/mic` in a chat: records until Enter (or Ctrl-C to drop it), shows
  `Listening… 4s` on the Thinking line's thread, transcribes, and puts the
  text **in the prompt for the person to read and edit**, never sent on
  its own: a misheard word must not run a command.
- Settings, in the key file like the vision model's:
  `RAK_STT_PROVIDER` (groq or openai), `RAK_STT_MODEL`, and
  `RAK_STT_COMMAND` for a command of the person's own that takes a WAV
  path and prints text (whisper.cpp, a script). With none set, `/mic` says
  what to set and points at macOS dictation.
- `RAK_MIC_COMMAND` for a recorder of the person's own; otherwise the
  first of ffmpeg, rec, arecord found on PATH.
- The recording is a temporary 16 kHz mono WAV, deleted after
  transcription; nothing is kept, logged or sent anywhere but the chosen
  transcription provider. Said in docs/safety.md.
- Language: left to the model to detect; `RAK_STT_LANGUAGE` (ar, en) to
  fix it when detection is wrong.

**Where the code goes.** `src/stt.rs` (lib: the multipart request, reading
the answer, the settings; pure parts unit-tested), `src/cli/mic.rs` (the
recorder process, the Listening line, Enter/Ctrl-C), a `/mic` row in
commands.rs, the settings in keys_command's KNOWN and .env.example.

**To check before building.**
- That Groq's and OpenAI's transcription endpoints still take a WAV in a
  multipart form and answer `{"text": …}` (read their docs then).
- Stopping ffmpeg cleanly so the WAV header is written: SIGINT to its
  process group, not SIGKILL.
- How it behaves over SSH (no mic there: say so, don't hang).
- Arabic quality on the chosen model, with real speech.

**Later, only if /mic is used:** a push-to-talk key in the editor; spoken
answers (text to speech) are a separate idea and not planned.

**Done when:** `/mic` on macOS and Linux turns a spoken sentence into text
in the prompt with Groq and with a local command; no recorder and no
provider each give a one-line answer saying what to set; the binary's size
and its dependency list are unchanged.

## From maki (github.com/tontinton/maki, read 2026-11-14)

- [x] Folder trust: a project's .rak/mcp.json, .rak/skills, .rak/routines and
      a .env with rak settings are used only once the folder is trusted
      (asked once in a terminal, kept in ~/.config/rak/trusted; --trust for
      one run). Before this, opening a cloned repo started its MCP programs
      and a project .env RAK_BASE_URL received the provider key.
- [x] run_shell keeps the start and the end of long output (a third and two
      thirds), the middle counted, instead of only the start.
- [ ] Later: keep a tool's partial output when Esc/Ctrl-C stops it (with the
      Esc item below); `code_execution` (tools called from a sandboxed
      script, many reads in one turn) is worth a look.

## Project notes in rak/ (from Naseem, built 2026-11-13, released in alpha.7)

The owner asked to learn from Naseem's `naseem/` folder. Built in
`src/notes.rs`: a visible `rak/` folder with project.md, roadmap.md,
journal.md (+ journal-archive.md), rules.md and memory.md (moved from
`.rak/`, the old file still read). Rules (rak/rules.md, AGENTS.md, CLAUDE.md)
and the notes are loaded into the prompt at the start; after a request that
changed something, one side call writes the journal line and MOVE/ADD
roadmap edits, applied without deleting; a `## Gate` command must run after
the last edit before "done" (sent back twice at most). Two differences from
Naseem: card matching ignores backticks and quotes (a live run dropped
them), and the on-screen summary counts only edits applied. Live-checked
with ollama-cloud: notes set up, gate sent the model back to run the
command, the Known issues card moved to Done with its date.

Added after alpha.7 (owner: a codebase check did not create rak/): `/init`
writes the notes from the project (or updates them), the no-notes prompt
line suggests it, and the update call is told which files the request
changed (a refused run's journal had said "no files created").

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
- [x] 9. Record and replay (SAK `SwiftAgentKitReplay`): capture a live run's
      replies to a file and replay it in tests.
- [x] 10. Parallel helpers (SAK `SubAgentGate`, `maxSubAgentConcurrency`;
      Naseem's `subAgentConcurrency`): several `delegate` calls in one reply
      run at the same time, each on its own thread, at most as many as the
      helper's provider takes: 3 on a commercial cloud API, 2 on Ollama's
      cloud (it slowed to timeouts with three, Naseem 2026-09-08), 1 on a
      model on this machine. The rest wait their turn. Helpers stay
      read-only, so two never edit one file. The main thread prints every
      helper's steps, tagged `[1]`, `[2]`… when more than one runs, and asks
      the fallback questions for them; Ctrl-C stops them all. Results go back
      in the order the model asked. Not ported: SAK's helpers that can edit
      (they would need approvals from several threads at once). Built
      2026-11-14; live: kimi-k2.6:cloud ran two helpers side by side on
      rak's own code, steps interleaved, both answers right.

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
