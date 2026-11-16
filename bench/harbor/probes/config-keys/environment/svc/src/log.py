import os

from svc.config import cfg


def load():
    value = os.environ.get("SVC_LOG_LEVEL", "info")
    value = cfg.get("log.format", "json")
    return locals()


def helper_9(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
