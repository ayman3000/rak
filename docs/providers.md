# Providers and models

Who runs the model, how keys are kept apart, what happens when a provider fails, and prompt caching with Claude. Back to the [README](../README.md).

**Contents:** [The providers](#the-providers) · [Choosing a provider and model](#choosing-a-provider-and-model) · [Keys and settings](#keys-and-settings) · [When a provider fails](#when-a-provider-fails) · [Claude and prompt caching](#claude-and-prompt-caching)

## The providers

rak knows these providers by name. Each gets its own address and only its own key:

| `RAK_PROVIDER` / `--provider` | Who | Key | Tried for real |
|---|---|---|---|
| `ollama` (the default) | Ollama on this machine, its `:cloud` models included | none | ✅ 2026-11-12 |
| `ollama-cloud` | Ollama's cloud at ollama.com, without Ollama installed | `OLLAMA_API_KEY` | ✅ 2026-11-12 |
| `anthropic` | Claude through Anthropic's API, with prompt caching | `ANTHROPIC_API_KEY` | ✅ 2026-11-12 |
| `openai` | OpenAI | `OPENAI_API_KEY` | not yet |
| `gemini` | Google Gemini | `GEMINI_API_KEY` | not yet |
| `zai`, `zai-coding` | Z.ai (GLM); `zai-coding` for GLM Coding Plan keys | `ZAI_API_KEY` | not yet |
| `openrouter` | OpenRouter | `OPENROUTER_API_KEY` | not yet |
| `huggingface` | Hugging Face's router | `HF_TOKEN` | not yet |
| `groq` | Groq | `GROQ_API_KEY` | not yet |
| `deepseek` | DeepSeek | `DEEPSEEK_API_KEY` | not yet |
| `mistral` | Mistral | `MISTRAL_API_KEY` | not yet |
| `lmstudio`, `llamacpp`, `vllm` | those servers on this machine | none | not yet |

"Not yet" means set up from the provider's documentation but no real request made. Tried one? Tell us how it went, in an issue. The list is data, [`src/providers.json`](../src/providers.json): adding a provider is one line there.

## Choosing a provider and model

| Where | How | Lasts |
|---|---|---|
| The key file | `RAK_PROVIDER=…` and `RAK_MODEL=…` | until you change it |
| The command line | `rak --provider groq -m MODEL "…"` | that run |
| In a chat | `/provider NAME`, `/model NAME` | that chat, and the next chats in that folder |

**Reasoning effort** works the same way: `RAK_EFFORT=high` in the key file, `--effort high` for one run (it wins over the file), `/effort high` in a chat (kept for that folder, see below). It is sent as `reasoning_effort`, or as Anthropic's `output_config.effort`. `xhigh` and `max` are not taken by every model.

With nothing set, a chat starts at `medium`, and a one-off run (`rak "…"`, `--json`) sends none, so scripts and benchmarks get the model's own default. If a model's server refuses the chat's `medium` (a 400), rak asks once more without it and sends that model none for the rest of the run. An effort you set yourself is never dropped: the server's answer is shown instead. `default` (in `RAK_EFFORT`, `--effort` or `/effort`) sends none, in a chat too.

**A folder remembers its pick.** When you switch with `/provider`, `/model` or `/effort`, rak notes it in the project folder as `.rak/model`: the provider's name, the model's name and the effort, never a key or an address. The next chat started in that folder begins there. `--provider`, `--base-url`, `-m`, `--effort`, and `RAK_PROVIDER` or `RAK_BASE_URL` in the environment win over it for that run. A one-off run (`rak "…"`) does not read it. If the provider's key is no longer set, rak says so and starts with your default. The file is plain text: commit it to share the pick with a team, or add `.rak/model` to `.gitignore` to keep it to yourself. In a folder you have not trusted (see [Safety](safety.md)) it is neither read nor written.

**Which model is used:** in a chat, `-m` if given, then the model picked last in this folder (when the chat is at the provider it was picked at), then the model you last picked for that provider anywhere, then `RAK_MODEL`, then a list to pick from. A one-off run (`rak "…"`) uses `-m`, then `RAK_MODEL`, then your last pick, so scripts stay predictable. `RAK_MODEL` belongs to the provider in `RAK_PROVIDER`, not to one chosen with `--provider`.

```bash
# Ollama on this machine: nothing to set
rak -m kimi-k2.6:cloud "explain what this project does"

# Another provider, for one run
rak --provider openrouter -m some/model "add a --verbose flag to the CLI"

# With ANTHROPIC_API_KEY set, a claude-* model goes to Anthropic on its own
rak -m claude-haiku-5-5 "summarize the README"

# A server rak doesn't know by name: its address, and RAK_API_KEY if it needs one
rak --base-url https://example.com/v1 -m some-model "…"
```

## Keys and settings

rak reads the environment first, then `./.env` in the project, then `~/.config/rak/.env`; the first one with a value wins. Within one file, the last line set wins, and an empty line erases nothing. `rak keys` and the chat's welcome panel show what is set and where it came from, never a key itself, and say what was set more than once.

| Setting | For |
|---|---|
| a provider's key (table above) | that provider, and only that provider |
| `TAVILY_API_KEY` | `web_search` |
| `RAK_PROVIDER`, `RAK_MODEL` | the selected provider and model |
| `RAK_EFFORT` | the reasoning effort: `low`, `medium`, `high`, `xhigh` or `max` (empty: the model's own) |
| `RAK_BASE_URL` | the selected provider at another address, e.g. Ollama on another machine |
| `RAK_API_KEY` | the key for a server named only by `--base-url` |

**A key never reaches another provider's server.** Defaults from the file never mix with flags: `--provider` uses that provider's own address even when `RAK_BASE_URL` is set, and `--base-url` drops the selected provider and its key. Values read from the files stay inside rak: commands the agent runs don't see them.

## When a provider fails

rak carries on with another provider instead of stopping, with nothing to set up:

1. **The same model is asked again first**, twice, when that can help (network trouble, a busy or rate-limited server). Out of credit, a bad key or an unknown model move on at once.
2. **Then the next provider:** each other provider whose key is set, in the order the key lines appear in your key file, then the servers on this machine, each with the model you last picked for it. A provider you have never used in rak is skipped, so a key sitting in the file is never spent on its own.
3. **In a chat, rak asks first:** `ollama-cloud · kimi-k2.6 failed: … Continue with anthropic · claude-sonnet-5-5? [y/n]`. With `-y`, or in a script, it switches and says so.
4. **The step is done again** on the new model with the conversation so far. Your next message goes back to the provider you chose, unless that one can't work at all (a bad key, a missing model).

**A silent server counts as a failure.** A streamed answer that sends nothing for too long is stopped and retried: 5 minutes for Anthropic, 10 for other cloud servers, 15 for a server on this machine, a minute or two more for a very long prompt (from SwiftAgentKit's `StreamStallPolicy`).

Every switch is shown (`↪ now using …`), logged with `--log`, listed in `--json` (`fell_back_to`) and counted in `/usage`. A refusal, a stop, or a request rak itself built wrongly does not fall back: another model would not do better.

## Claude and prompt caching

An agent sends the whole conversation again with every call, so most of what it pays for is input it sent before. Prompt caching lets the server read that part back for a fraction of the price: on Anthropic's API a cache read costs a tenth of normal input, or less.

With `--provider anthropic` (or a `claude-*` model and `ANTHROPIC_API_KEY` set), rak speaks Anthropic's Messages API directly:

- **Caching** marks the system prompt (which covers the tools) and the end of the conversation, so each call reads back everything the previous one sent.
- **Thinking blocks** are sent back unchanged, as the API requires.
- **Refusals:** for the models that can decline a request, rak asks for Anthropic's server-side fallback, so a decline is answered by the recommended model in the same call.
- **The context window** is read from Anthropic's models list.

After each request rak shows how much input came from the cache (`6220 tokens in (3000 cached)`). Other servers that report cached tokens (Ollama's cloud models do) show them the same way.
