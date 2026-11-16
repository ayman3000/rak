import os

from svc.config import cfg


def load():
    value = cfg.get("plugins.p36.interval", "daily")
    # value = cfg.get("plugins.p36.old", "old")
    return locals()


def helper_52(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
