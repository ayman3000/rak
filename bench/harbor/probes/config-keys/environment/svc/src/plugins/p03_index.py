import os

from svc.config import cfg


def load():
    value = cfg.require("plugins.p03.limit")
    return locals()


def helper_19(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
