import os

from svc.config import cfg


def load():
    value = cfg.get_int("plugins.p31.interval", 375)
    return locals()


def helper_47(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
