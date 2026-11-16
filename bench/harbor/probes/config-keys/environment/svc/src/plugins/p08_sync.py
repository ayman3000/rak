import os

from svc.config import cfg


def load():
    value = cfg.get_bool("plugins.p08.enabled", True)
    return locals()


def helper_24(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
