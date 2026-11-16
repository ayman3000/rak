import os

from svc.config import cfg


def load():
    workers = cfg.get_int("jobs.workers", 4)
    queue = cfg.get("jobs.queue", "default")
    return locals()


def helper_7(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
