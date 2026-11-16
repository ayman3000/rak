import os

from svc.config import cfg


def load():
    value = cfg.require("plugins.p15.enabled")
    return locals()


def helper_31(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
