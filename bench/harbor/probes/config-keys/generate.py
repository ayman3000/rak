"""Writes the config-keys probe: a service whose configuration is read in
about 60 files, in several styles (environment/svc), the expected answer
(tests/expected.json) and the reference solution (solution/solve.sh). Run it
again after changing the tables below; the output is checked in.
"""

import json
import random
import shutil
from pathlib import Path

HERE = Path(__file__).parent
SRC = HERE / "environment" / "svc" / "src"

# One read: (code, key, default as the answer shows it, or None for "(none)").
# A key of None is a decoy that must not be counted.
def get(key, default):
    return (f'value = cfg.get("{key}", "{default}")', key, default if default != "" else '""')


def get_int(key, default):
    return (f'value = cfg.get_int("{key}", {default})', key, str(default))


def get_bool(key, default):
    return (f'value = cfg.get_bool("{key}", {default})', key, str(default))


def require(key):
    return (f'value = cfg.require("{key}")', key, None)


def subscript(key):
    return (f'value = cfg["{key}"]', key, None)


def env(key, default=None):
    if default is None:
        return (f'value = os.getenv("{key}")', key, None)
    return (f'value = os.environ.get("{key}", "{default}")', key, default)


def env_subscript(key):
    return (f'value = os.environ["{key}"]', key, None)


def multiline(key, default):
    return (f'value = cfg.get(\n        "{key}",\n        "{default}",\n    )', key, default)


def constant(key, default):
    name = key.upper().replace(".", "_")
    return (f'{name} = "{key}"\n    value = cfg.get_int({name}, {default})', key, str(default))


def commented(key):
    return (f'# value = cfg.get("{key}", "old")', None, None)


def in_string(key):
    return (f'value = "use cfg.get(\\"{key}\\", 1) to read it"', None, None)


READS = {
    "app.py": [get("server.host", "0.0.0.0"), get_int("server.port", 8080)],
    "db/__init__.py": [require("db.url")],
    "db/pool.py": [get_int("db.pool_size", 10), constant("db.timeout", 30)],
    "db/migrate.py": [require("db.url"), env("SVC_MIGRATE_DRY")],
    "api/auth.py": [subscript("api.token"), get_int("api.token_ttl", 3600)],
    "api/routes.py": [get("api.prefix", "/v1"), get("server.host", "0.0.0.0")],
    "api/limits.py": [get_int("api.rate_limit", 100), commented("api.burst")],
    "jobs/worker.py": [get_int("jobs.workers", 4), multiline("jobs.queue", "default")],
    "jobs/schedule.py": [env("SVC_TZ", "UTC"), get("jobs.cron", "*/5 * * * *")],
    "log.py": [env("SVC_LOG_LEVEL", "info"), get("log.format", "json")],
    "cache.py": [get_int("cache.ttl", 300), env("SVC_CACHE_URL")],
    "mail/send.py": [get("mail.host", "smtp.local"), require("mail.from")],
    "mail/templates.py": [get("mail.templates", "templates/mail")],
    "metrics.py": [env("SVC_METRICS", "off"), get_int("metrics.port", 9100)],
    "util/feature.py": [get("features", ""), commented("features.legacy")],
    "util/text.py": [in_string("text.width")],
    "secrets.py": [env_subscript("SVC_SECRET_KEY")],
}

# Plugins: many small modules, so the survey has to cover a lot of files.
rng = random.Random(11)
STYLES = [get, get_int, get_bool, require, multiline, constant]
for i in range(1, 41):
    name = f"plugins/p{i:02d}_{rng.choice(['sync', 'export', 'audit', 'hooks', 'index'])}.py"
    style = STYLES[i % len(STYLES)]
    key = f"plugins.p{i:02d}.{rng.choice(['enabled', 'interval', 'target', 'limit'])}"
    if style in (get, multiline):
        reads = [style(key, rng.choice(["on", "daily", "/tmp/out", "eu-west"]))]
    elif style in (get_int, constant):
        reads = [style(key, rng.randint(1, 500))]
    elif style is get_bool:
        reads = [style(key, rng.choice([True, False]))]
    else:
        reads = [style(key)]
    if i % 7 == 0:
        reads.append(get_int("server.port", 8080))
    if i % 9 == 0:
        reads.append(commented(f"plugins.p{i:02d}.old"))
    READS[name] = reads

FILLER = '''

def helper_{n}(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
'''


def expected():
    keys = {}
    for path, reads in READS.items():
        for _, key, default in reads:
            if key is None:
                continue
            entry = keys.setdefault(key, {"default": default or "(none)", "files": set()})
            entry["files"].add(path)
    return {k: {"default": v["default"], "files": sorted(v["files"])} for k, v in sorted(keys.items())}


def main():
    if SRC.exists():
        shutil.rmtree(SRC)
    for n, (path, reads) in enumerate(READS.items()):
        file = SRC / path
        file.parent.mkdir(parents=True, exist_ok=True)
        body = "import os\n\nfrom svc.config import cfg\n\n\ndef load():\n"
        body += "".join(f"    {code}\n" for code, _, _ in reads)
        body += "    return locals()\n" + FILLER.format(n=n)
        file.write_text(body)
    (SRC / "README.md").write_text('Configuration is read with cfg.get("name", default) and cfg.require("name").\n')
    answer = expected()
    tests = HERE / "tests"
    tests.mkdir(exist_ok=True)
    (tests / "expected.json").write_text(json.dumps(answer, indent=1) + "\n")
    table = "| key | default | files |\n|---|---|---|\n" + "".join(
        f"| {k} | {v['default']} | {', '.join(v['files'])} |\n" for k, v in answer.items()
    )
    solution = HERE / "solution"
    solution.mkdir(exist_ok=True)
    (solution / "solve.sh").write_text("#!/bin/bash\ncat > /app/KEYS.md <<'EOF'\n" + table + "EOF\n")
    print(len(READS), "files,", len(answer), "keys")


if __name__ == "__main__":
    main()
