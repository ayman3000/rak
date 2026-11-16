import os

from svc.config import cfg


def load():
    value = cfg.get_bool("plugins.p02.interval", True)
    return locals()


def helper_18(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
