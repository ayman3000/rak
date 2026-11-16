import os

from svc.config import cfg


def load():
    value = cfg.get_bool("plugins.p32.limit", False)
    return locals()


def helper_48(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
