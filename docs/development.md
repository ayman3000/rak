# Development

Using rak as a library, how the code is laid out, and how it is checked and released. Back to the [README](../README.md).

**Contents:** [As a library](#as-a-library) · [Building and testing](#building-and-testing) · [How the code is laid out](#how-the-code-is-laid-out) · [Checks and releases](#checks-and-releases)

## As a library

```rust
use rak::agent::{system_prompt, Agent, Config};
use rak::provider::OpenAICompatible;
use rak::tools::{core_tools, Ctx};

let cwd = std::env::current_dir()?;
let ctx = Ctx::new(&cwd, false);
let model = OpenAICompatible::new("http://localhost:11434/v1", None, "kimi-k2.6:cloud");
let config = Config { system_prompt: system_prompt(&ctx.cwd, false, None), max_turns: 30, context_window: 128_000 };
let mut agent = Agent::new(Box::new(model), core_tools(&ctx, false), config);
let result = agent.run("Make the tests pass.");
println!("{}", result.answer);
```

A runnable version is in [examples/minimal.rs](../examples/minimal.rs). Add a tool by implementing `rak::tools::Tool`; plug in another model API by implementing `rak::provider::Model`.

## Building and testing

With Rust 1.88 or newer:

```bash
cargo test
cargo build --release
```

## How the code is laid out

The code is in two layers: a library that knows nothing about terminals, and the `rak` command on top of it. Dependencies only point down; nothing in the library uses `cli`.

| Library (`src/`) | |
|---|---|
| `agent.rs` | The loop: ask the model, run the tools it calls, repeat. Retries, loop detection, the repair nudge, the finish and gate checks, trimming. |
| `compact.rs`, `artifacts.rs` | Summarizing a full context; output set aside when trimmed. |
| `provider.rs`, `anthropic.rs` | The `Model` trait and the OpenAI-compatible client; Claude's Messages API. |
| `providers.rs`, `providers.json`, `fallback.rs`, `stall.rs` | The named providers; moving to another when one fails; the silent-server limits. |
| `tools/` | The `Tool` trait and the built-in tools (`fs`, `patch`, `shell`, `map`, `todo`, `image`, `web`, `fetch`); `verify` is the write check. |
| `delegate.rs` | The helper agent run inside a tool call. |
| `notes.rs`, `memory.rs`, `routines.rs`, `sessions.rs` | The `rak/` notes and gate; `remember`; routines; saved chats. |
| `mcp/`, `skills/` | MCP config, client and bridge; reading, saving and fetching skills. |
| `replay.rs` | `--record` and `--replay`. |
| `private_file.rs` | Writing files that hold keys, readable by their owner only. |
| `interrupt.rs`, `read_slice.rs`, `text.rs` | The stop flag, the file-read window, text helpers. |

| Command (`src/cli/`) | |
|---|---|
| `args`, `session`, `chain` | The command line; one run set up and reported; the fallback chain's questions. |
| `chat`, `commands` | The chat loop and its slash commands. |
| `approval`, `output`, `log` | Asking before changes; what is shown; `--log`. |
| `mcp_command`, `skills_command`, `keys_command`, `setup` | `rak mcp`, `rak skills`, `rak keys`, and the first-run setup (`rak setup`). |
| `editor`, `menu`, `picker`, `markdown`, `welcome`, `style`, `term` | The interface's parts, each pure where it can be, and tested. |
| `settings`, `signals`, `trust` | What is kept between runs; Ctrl-C; folder trust. |

## Checks and releases

Every push to `main` and every pull request runs the tests on Linux and macOS, the format and lint checks, and a build with the oldest supported Rust (`.github/workflows/ci.yml`). A version tag matching `Cargo.toml` builds the four release archives and publishes them (`.github/workflows/release.yml`); a tag with a `-` makes a pre-release:

```bash
git tag v0.1.0-alpha.13 && git push origin v0.1.0-alpha.13
```
