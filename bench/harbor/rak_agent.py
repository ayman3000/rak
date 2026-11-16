"""rak as a Harbor installed agent, for Terminal-Bench 2.1 and DeepSWE.

Installs a pinned rak release in the task's container with rak's own
install.sh and runs one unattended request:

    rak -y --json --base-url URL -m MODEL --time-limit SECS -- "<task>"

--time-limit is the task's own agent timeout, read from its task.toml, so
the model knows how long it has and is warned before Harbor stops it.

An unattended run (one request with -y) has no turn limit, keeps nothing for
later sessions and writes no files of its own into the project. The model is
reached through the base URL Harbor resolves (`--base-url`), with the key, when
there is one, in RAK_API_KEY.

Options (`--ak key=value`; see `harbor agent schema`):
    version           rak release tag (default below)
    context           the model's context window in tokens   (rak --context)
    max_tokens        the model's longest reply               (rak --max-tokens)
    reasoning_effort  low, medium, high (also --effort)       (rak --effort)
    review            true: review before the answer (rak --review; opt-in from alpha.20)
    no_review         true: skip it on alpha.19, where it was on by default (rak --no-review)

Written for Harbor 0.26 (BaseInstalledAgent with ModelConnectionSpec).
"""

import json
import shlex
import tomllib
from pathlib import Path
from typing import Annotated, override

from pydantic import Field

from harbor.agents.agent_auth_resolution import ModelConnectionSpec
from harbor.agents.capabilities import AgentCapabilities
from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.agents.options import Cli, InstalledAgentOptions
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.model_api_format import ModelApiFormat

RAK_DEFAULT_VERSION = "v0.1.0-alpha.19"
RAK_INSTALL_URL = "https://raw.githubusercontent.com/ayman3000/rak/main/install.sh"
PATH_PREFIX = 'export PATH="$HOME/.local/bin:$PATH"'


class RakOptions(InstalledAgentOptions):
    context: Annotated[int | None, Cli("--context")] = Field(
        default=None, description="The model's context window in tokens."
    )
    max_tokens: Annotated[int | None, Cli("--max-tokens")] = Field(
        default=None, description="The model's longest reply in tokens."
    )
    reasoning_effort: Annotated[str | None, Cli("--effort")] = Field(
        default=None, description="Reasoning effort: low, medium or high."
    )
    review: Annotated[bool | None, Cli("--review")] = Field(
        default=None, description="Review the work against the request before answering (alpha.20 and later)."
    )
    no_review: Annotated[bool | None, Cli("--no-review")] = Field(
        default=None, description="Skip the review before the answer (alpha.19, where it was on by default)."
    )


class RakAgent(BaseInstalledAgent):
    """rak, a terminal coding agent in Rust: https://github.com/ayman3000/rak"""

    capabilities = AgentCapabilities(base_url=True, reasoning_effort=True)
    MODEL_CONNECTION = ModelConnectionSpec(
        supported_api_formats=[ModelApiFormat.OPENAI_CHAT_COMPLETIONS],
        # rak takes the address as a flag, not from the environment.
        base_url_destination_names={"default": None},
        api_key_destination_names={"default": "RAK_API_KEY"},
    )
    options_model = RakOptions

    @classmethod
    @override
    def check_model_credentials(cls, connection, *, kwargs, env, explicit_env=None):
        # A local server (Ollama) needs no key; a provider that does says so
        # itself on the first request.
        return None

    @staticmethod
    @override
    def name() -> str:
        return "rak"

    @override
    def get_version_command(self) -> str | None:
        return f"{PATH_PREFIX}; rak --version"

    @override
    def parse_version(self, stdout: str) -> str:
        return stdout.strip().removeprefix("rak").strip()

    def _release(self) -> str:
        return self._version or RAK_DEFAULT_VERSION

    @override
    async def install(self, environment: BaseEnvironment) -> None:
        # rak's install.sh downloads with curl or wget. curl is tried first:
        # wget alone once failed a release download (exit 5, TLS) on
        # mteb-leaderboard's image. An image that cannot install curl but has
        # wget (qemu-alpine-ssh's Debian 11: the security archive answers 404)
        # goes on with wget.
        try:
            await self.ensure_system_dependencies(environment, ("curl", "tar"))
        except Exception:
            has_wget = await environment.exec(command="command -v wget >/dev/null 2>&1", user="root")
            if has_wget.return_code != 0:
                raise
            await self.ensure_system_dependencies(environment, ("tar",))
        fetch = (
            f"{{ curl -fsSL {RAK_INSTALL_URL} 2>/dev/null || wget -qO- {RAK_INSTALL_URL}; }}"
        )
        await self.exec_as_agent(
            environment,
            command=(
                f"{fetch} | RAK_VERSION={shlex.quote(self._release())} sh && "
                f"{PATH_PREFIX} && rak --version"
            ),
        )

    def _model(self) -> str:
        # Harbor model names may carry a provider prefix ("openai/x"); behind a
        # base URL rak wants the name the server itself knows.
        name = self.model_name or ""
        if self.skip_model_parsing or "/" not in name:
            return name
        return name.split("/", 1)[1]

    @override
    @with_prompt_template
    async def run(self, instruction: str, environment: BaseEnvironment, context: AgentContext) -> None:
        access = self.model_connection
        env = dict(access.env)
        if access.api_key:
            env["RAK_API_KEY"] = access.api_key
        flags = ["-y", "--json"]
        base_url = access.execution_base_url or self._base_url
        if base_url:
            flags += ["--base-url", shlex.quote(base_url)]
        if self.model_name:
            flags += ["-m", shlex.quote(self._model())]
        limit = self._time_limit() if self._has_time_limit() else None
        if limit:
            flags += ["--time-limit", str(limit)]
        extra = self.build_cli_flags()
        if extra:
            flags.append(extra)
        logs = self.environment_logs_dir.as_posix()
        # The answer and numbers (--json) go to one file, the step-by-step
        # progress (stderr) to another, and every step as JSON lines to a log.
        # "--" ends the options, so a task that starts with "-" (a Markdown
        # list) is still read as the task, not as an unknown option.
        await self.exec_as_agent(
            environment,
            command=(
                f"mkdir -p {logs} && {PATH_PREFIX} && "
                f"rak {' '.join(flags)} --log {logs}/rak-log.jsonl -- {shlex.quote(instruction)} "
                f"> {logs}/rak-result.json 2> {logs}/rak-progress.txt"
            ),
            env=env,
        )

    def _has_time_limit(self) -> bool:
        # --time-limit came in v0.1.0-alpha.16; an older release (for a
        # before/after comparison) would stop at the unknown option.
        release = self._release()
        if "-alpha." not in release:
            return True
        try:
            return int(release.rsplit(".", 1)[1]) >= 16
        except ValueError:
            return True

    def _time_limit(self) -> int | None:
        # Harbor stops the agent at the task's timeout but does not tell an
        # installed agent what it is. The trial's config.json (next to the
        # agent's logs) names the task; its task.toml, in Harbor's task cache
        # or the task's own folder, has [agent] timeout_sec. Any doubt: None,
        # and rak runs without a limit as before.
        try:
            config = json.loads((self.logs_dir.parent / "config.json").read_text())
            task = config["task"]
            if task.get("path"):
                toml_file = Path(task["path"]) / "task.toml"
            else:
                digest = task["ref"].split(":", 1)[1]
                toml_file = Path.home() / ".cache/harbor/tasks/packages" / task["name"] / digest / "task.toml"
            seconds = tomllib.loads(toml_file.read_text())["agent"]["timeout_sec"]
            agent = config.get("agent") or {}
            if agent.get("override_timeout_sec"):
                seconds = agent["override_timeout_sec"]
            if agent.get("max_timeout_sec"):
                seconds = min(seconds, agent["max_timeout_sec"])
            multiplier = config.get("agent_timeout_multiplier") or config.get("timeout_multiplier") or 1.0
            return int(seconds * multiplier) or None
        except (OSError, ValueError, KeyError, IndexError, TypeError, tomllib.TOMLDecodeError):
            return None

    def _summary_from_log(self) -> dict | None:
        # A run Harbor stopped at its time limit never prints its summary;
        # every model call is in the step log, so the tokens are added up there.
        log = self.logs_dir / "rak-log.jsonl"
        if not log.exists():
            return None
        totals = {"input_tokens": 0, "cached_tokens": 0, "output_tokens": 0}
        for line in log.read_text().splitlines():
            try:
                step = json.loads(line)
            except ValueError:
                continue
            if step.get("event") == "model_call":
                for key in totals:
                    totals[key] += step.get(key) or 0
        return totals

    @override
    def populate_context_post_run(self, context: AgentContext) -> None:
        # rak --json prints one object: outcome, answer, turns, tool_calls,
        # input_tokens, cached_tokens, output_tokens, seconds, fell_back_to.
        summary = None
        result = self.logs_dir / "rak-result.json"
        if result.exists():
            try:
                summary = json.loads(result.read_text().strip().splitlines()[-1])
            except (ValueError, IndexError):
                summary = None
        summary = summary or self._summary_from_log()
        if not summary:
            return
        for field, key in (
            ("n_input_tokens", "input_tokens"),
            ("n_cache_tokens", "cached_tokens"),
            ("n_output_tokens", "output_tokens"),
        ):
            if hasattr(context, field) and key in summary:
                setattr(context, field, summary[key])
