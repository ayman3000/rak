import os

from svc.config import cfg


def load():
    value = cfg.require("plugins.p27.interval")
    # value = cfg.get("plugins.p27.old", "old")
    return locals()


def helper_43(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
