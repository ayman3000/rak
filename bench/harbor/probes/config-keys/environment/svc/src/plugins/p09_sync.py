import os

from svc.config import cfg


def load():
    value = cfg.require("plugins.p09.interval")
    # value = cfg.get("plugins.p09.old", "old")
    return locals()


def helper_25(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
