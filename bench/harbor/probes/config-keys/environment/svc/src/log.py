import os

from svc.config import cfg


def load():
    level = os.environ.get("SVC_LOG_LEVEL", "info")
    fmt = cfg.get("log.format", "json")
    return locals()


def helper_9(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
