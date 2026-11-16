import os

from svc.config import cfg


def load():
    value = cfg.get_int("plugins.p19.target", 198)
    return locals()


def helper_35(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
