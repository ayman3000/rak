import os

from svc.config import cfg


def load():
    value = cfg.get_int("plugins.p37.enabled", 22)
    return locals()


def helper_53(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
