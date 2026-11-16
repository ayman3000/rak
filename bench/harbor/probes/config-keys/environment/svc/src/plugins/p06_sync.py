import os

from svc.config import cfg


def load():
    value = cfg.get("plugins.p06.enabled", "eu-west")
    return locals()


def helper_22(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
