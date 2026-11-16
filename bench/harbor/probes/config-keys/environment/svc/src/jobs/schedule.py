import os

from svc.config import cfg


def load():
    tz = os.environ.get("SVC_TZ", "UTC")
    cron = cfg.get("jobs.cron", "*/5 * * * *")
    return locals()


def helper_8(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
