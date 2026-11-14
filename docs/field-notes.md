# Field notes

What happened when rak's features were tried by hand with real models. These are a handful of tries, not measurements; they explain why some prompt lines and checks exist. Back to the [README](../README.md).

## Writing skills with the agent

Tried with kimi-k2.6:cloud (2026-11-11), in a test folder:

- **Asked for "a skill for writing git commit messages the way my team does":** it loaded `Write a skill` and asked three questions. After the answers it showed a draft, saved it when told to, and the file was there.
- **Corrected after recovering on its own:** told to run `python3 test_app.py`, it saw the error and switched to `./check.sh`. The user then said "Next time don't run the file directly: in this project tests always go through `./check.sh`, because it sets APP_ENV first". It saved a project skill holding the rule and the reason. A new session asked to "run the tests" loaded that skill and ran `./check.sh` first.
- **Corrected after a refusal:** the user said no to `APP_ENV=test python3 test_app.py` and wrote "No. In this project tests always go through ./check.sh…". In three tries, each with a different wording of the prompt line, it ran `./check.sh` but kept nothing. With the reminder added to what the model is told after a no, the same exchange made it save the lesson unasked (one try).
- **A plain correction with no refusal and no "next time"** was tried once, before the prompt line existed, and nothing was kept.

These are a handful of tries with one model, not a measurement. A lesson was kept when the user said no to a step or said the correction was for next time. If a correction matters, you can also just ask the agent to save it.

## The iOS app skill

[`skills/ios-app.md`](skills/ios-app.md) builds an iOS app from the terminal and runs it in the Simulator, without an Xcode project: `swiftc` for the Simulator, `simctl` to install, launch and screenshot it, and a Swift package for logic tests. Without it, gemma4:31b-cloud asked to "create an Xcode weather project with one nice screen and test it" invented a command that does not exist (`xcodebuild -create-project`), then gave up on the Simulator and told the user to finish in Xcode. With it, the same model built the app, fixed its own compile error, launched it in the Simulator, ran its test, and reported which checks it made.

A second session, after the skill learned launch arguments and the app's log, and with `view_image`, asked the same model for a weather app for Egyptian cities with a Settings tab, using the free Open-Meteo API, tested on the Simulator. Before writing any code the model called the API with `curl` for three cities. It tested the decoding with `swift test`, opened the Settings screen through a launch argument (the app still starts on its main screen), and looked at both screenshots with `view_image`; its descriptions matched what was on screen. One claim went too far: it reported the fetches as verified from a log that printed "Verified" even when a fetch failed, from test code that ran on every launch. The skill now forbids both.

## Large projects

A line in the system prompt says when to use these. The four tools add 214 input tokens to every call (1,157 → 1,371 with skills off, kimi-k2.6:cloud).

Tried with kimi-k2.6:cloud (2026-11-11):

- **A multi-file build:** a library CLI in Python, with models, JSON storage, argparse commands and tests, done with `-y` in the benchmark's container. It kept a five-step task list from start to finish, ran its tests, and ran the CLI end to end. I ran its 7 tests again afterwards: they pass. Its first `todo` call named the field `name` instead of `text`; the tool now takes either.
- **A question about rak's own code** (about 45 files): "every condition under which this program talks to Anthropic's API, with file:line". Without the system-prompt line, it never used `code_map` (128,000 and 185,000 input tokens in two runs). With it, it started with `code_map` each time; runs took 88,000, 136,000 and 211,000 tokens, so the model's own choices vary more than the tools change. The answers were right every time.
- **`delegate`:** kimi did not choose it on its own, even for "summarize how every tool checks its inputs" (13 files read in its own context). Asked to use it, the helper found the right file; one line number was exact and one was off by five. The main conversation spent three turns.

Compaction was tried with kimi-k2.6:cloud (2026-11-12) by forcing a tiny window (`--context 4000`) on a read-only chat about rak's own files. A second request summarized the first ("summarizing 7 earlier messages"), and the model still named the three files the first request was about. In the first request there was nothing older to summarize, so the last-resort step replaced the file just read with its note; the model answered about that file from the note instead of reading it again, and got it wrong. With real windows (128K and up) a single read of at most 40,000 characters does not reach that step.

Not tried yet: a real codebase of thousands of files, a real session long enough to fill a full-size window, and `delegate` with Claude.

On 2026-11-12 deepseek-v4.1-flash chose `delegate` on its own for "check the codebase" (about 38 reads in the helper, 1.3M input tokens for the request, 93% cached). On 2026-11-14 kimi-k2.7-code used it when asked, reading 14 files in the helper while the main context went from 2.9K to 6.2K tokens.

## What has been tested where

Not yet tested: Windows (not built yet), and any public benchmark. Anthropic's real API was tried on Claude Haiku 5.5 (2026-11-11): the first call wrote 1,514 tokens to the cache and the second read all 1,514 back, writing only the 87 new ones. Its other paths (thinking blocks sent back, refusal fallbacks) are tested against a local stand-in. The chat and the MCP client were driven by hand on macOS only; on Linux they build and their unit tests pass. The MCP client was tried with one real server (the reference filesystem server, a program); web-address servers are covered by a unit test against a stand-in. Auto mode (`-y`) has only been run inside the benchmark's containers.
