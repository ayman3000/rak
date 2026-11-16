import os

from svc.config import cfg


def load():
    value = cfg.get_int("plugins.p13.enabled", 340)
    return locals()


def helper_29(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
