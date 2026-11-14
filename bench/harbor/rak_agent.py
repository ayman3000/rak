"""rak as a Harbor installed agent, for Terminal-Bench 2.1 and DeepSWE.

Installs a pinned rak release in the task's container with rak's own
install.sh and runs one unattended request:

    rak -y --json --base-url URL -m MODEL "<task>"

An unattended run (one request with -y) has no turn limit, keeps nothing for
later sessions and writes no files of its own into the project. The model is
reached through the base URL Harbor gives (`--base-url`), with the key in
RAK_API_KEY when the endpoint needs one.

Agent kwargs (`--ak key=value`):
    version     rak release tag (default below)
    context     the model's context window in tokens   (rak --context)
    max_tokens  the model's longest reply               (rak --max-tokens)
    effort      reasoning effort: low, medium, high     (rak --effort)

Written against Harbor's BaseInstalledAgent (install/run); not yet run.
"""

import json
import os
import shlex

from harbor.agents.capabilities import AgentCapabilities
from harbor.agents.installed.base import BaseInstalledAgent, with_prompt_template
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.trial.paths import EnvironmentPaths

RAK_DEFAULT_VERSION = "v0.1.0-alpha.11"
RAK_INSTALL_URL = "https://raw.githubusercontent.com/ayman3000/rak/main/install.sh"
PATH_PREFIX = 'export PATH="$HOME/.local/bin:$PATH"'


class RakAgent(BaseInstalledAgent):
    capabilities = AgentCapabilities(base_url=True)

    def __init__(self, *args, version=None, context=None, max_tokens=None, effort=None, **kwargs):
        super().__init__(*args, **kwargs)
        self._rak_version = version or RAK_DEFAULT_VERSION
        self._context = context
        self._max_tokens = max_tokens
        self._effort = effort

    @staticmethod
    def name() -> str:
        return "rak"

    async def install(self, environment: BaseEnvironment) -> None:
        await self.ensure_system_dependencies(environment, ("curl", "tar"))
        await self.exec_as_agent(
            environment,
            command=(
                f"curl -fsSL {RAK_INSTALL_URL} | RAK_VERSION={shlex.quote(self._rak_version)} sh && "
                f"{PATH_PREFIX} && rak --version"
            ),
        )

    def _flags(self) -> str:
        flags = ["-y", "--json"]
        if self._base_url:
            flags += ["--base-url", shlex.quote(self._base_url)]
        if self.model_name:
            # Harbor model names may carry a provider prefix ("openai/x");
            # behind a base URL rak wants the name the server knows.
            model = self.model_name.split("/", 1)[1] if self._base_url and "/" in self.model_name else self.model_name
            flags += ["-m", shlex.quote(model)]
        for flag, value in (("--context", self._context), ("--max-tokens", self._max_tokens), ("--effort", self._effort)):
            if value:
                flags += [flag, shlex.quote(str(value))]
        return " ".join(flags)

    @with_prompt_template
    async def run(self, instruction: str, environment: BaseEnvironment, context: AgentContext) -> None:
        env = {}
        if os.environ.get("RAK_API_KEY"):
            env["RAK_API_KEY"] = os.environ["RAK_API_KEY"]
        agent_dir = EnvironmentPaths.agent_dir.as_posix()
        # The answer and numbers (--json) go to one file, the step-by-step
        # progress (stderr) to another, and every step as JSON lines to a log.
        await self.exec_as_agent(
            environment,
            command=(
                f"mkdir -p {agent_dir} && {PATH_PREFIX} && "
                f"rak {self._flags()} --log {agent_dir}/rak-log.jsonl {shlex.quote(instruction)} "
                f"> {agent_dir}/rak-result.json 2> {agent_dir}/rak-progress.txt"
            ),
            env=env,
        )

    def populate_context_post_run(self, context: AgentContext) -> None:
        # rak --json prints one object: outcome, answer, turns, tool_calls,
        # input_tokens, cached_tokens, output_tokens, seconds, fell_back_to.
        result = self.logs_dir / "rak-result.json"
        if not result.exists():
            return
        try:
            summary = json.loads(result.read_text().strip().splitlines()[-1])
        except (ValueError, IndexError):
            return
        for field, key in (("n_input_tokens", "input_tokens"), ("n_cache_tokens", "cached_tokens"), ("n_output_tokens", "output_tokens")):
            if hasattr(context, field) and key in summary:
                setattr(context, field, summary[key])
