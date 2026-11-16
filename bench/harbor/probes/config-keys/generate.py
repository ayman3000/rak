"""Writes the config-keys probe: a small service whose configuration is read
in many files (environment/svc), the expected answer (tests/expected.json) and
the reference solution (solution/solve.sh). Run it again after changing the
tables below; the output is checked in.
"""

import json
from pathlib import Path

HERE = Path(__file__).parent
SRC = HERE / "environment" / "svc" / "src"

# file -> lines of code that read configuration (the rest of each file is filler)
READS = {
    "app.py": ['host = cfg.get("server.host", "0.0.0.0")', 'port = cfg.get_int("server.port", 8080)'],
    "db/__init__.py": ['url = cfg.require("db.url")'],
    "db/pool.py": ['size = cfg.get_int("db.pool_size", 10)', 'timeout = cfg.get_int("db.timeout", 30)'],
    "db/migrate.py": ['url = cfg.require("db.url")', 'dry = os.getenv("SVC_MIGRATE_DRY")'],
    "api/auth.py": ['token = cfg.require("api.token")', 'ttl = cfg.get_int("api.token_ttl", 3600)'],
    "api/routes.py": ['prefix = cfg.get("api.prefix", "/v1")', 'host = cfg.get("server.host", "0.0.0.0")'],
    "api/limits.py": ['rate = cfg.get_int("api.rate_limit", 100)', '# burst = cfg.get_int("api.burst", 20)'],
    "jobs/worker.py": ['workers = cfg.get_int("jobs.workers", 4)', 'queue = cfg.get("jobs.queue", "default")'],
    "jobs/schedule.py": ['tz = os.environ.get("SVC_TZ", "UTC")', 'cron = cfg.get("jobs.cron", "*/5 * * * *")'],
    "log.py": ['level = os.environ.get("SVC_LOG_LEVEL", "info")', 'fmt = cfg.get("log.format", "json")'],
    "cache.py": ['ttl = cfg.get_int("cache.ttl", 300)', 'url = os.getenv("SVC_CACHE_URL")'],
    "mail/send.py": ['host = cfg.get("mail.host", "smtp.local")', 'sender = cfg.require("mail.from")'],
    "mail/templates.py": ['folder = cfg.get("mail.templates", "templates/mail")'],
    "metrics.py": ['on = os.environ.get("SVC_METRICS", "off")', 'port = cfg.get_int("metrics.port", 9100)'],
    "util/feature.py": ['flags = cfg.get("features", "")', '# old = cfg.get("features.legacy", "x")'],
}

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
    for path, lines in READS.items():
        for line in lines:
            if line.lstrip().startswith("#"):
                continue
            call = line.split("=", 1)[1].strip()
            name = call.split("(", 1)[1].split(",")[0].split(")")[0].strip().strip('"')
            if "," in call:
                default = call.split(",", 1)[1].rsplit(")", 1)[0].strip().strip('"')
            else:
                default = "(none)"
            if default == "":
                default = '""'
            entry = keys.setdefault(name, {"default": default, "files": set()})
            entry["files"].add(path)
    return {k: {"default": v["default"], "files": sorted(v["files"])} for k, v in sorted(keys.items())}


def main():
    for n, (path, lines) in enumerate(READS.items()):
        file = SRC / path
        file.parent.mkdir(parents=True, exist_ok=True)
        body = "import os\n\nfrom svc.config import cfg\n\n\ndef load():\n"
        body += "".join(f"    {line}\n" for line in lines)
        body += "    return locals()\n" + FILLER.format(n=n)
        file.write_text(body)
    (SRC / "README.md").write_text("Configuration is read with cfg.get(\"name\", default) and cfg.require(\"name\").\n")
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
    print(len(answer), "keys")


if __name__ == "__main__":
    main()
