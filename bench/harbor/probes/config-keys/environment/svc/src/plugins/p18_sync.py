import os

from svc.config import cfg


def load():
    value = cfg.get("plugins.p18.enabled", "eu-west")
    # value = cfg.get("plugins.p18.old", "old")
    return locals()


def helper_34(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
