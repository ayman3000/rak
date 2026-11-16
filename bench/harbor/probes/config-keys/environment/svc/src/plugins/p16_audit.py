import os

from svc.config import cfg


def load():
    value = cfg.get(
        "plugins.p16.target",
        "daily",
    )
    return locals()


def helper_32(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
