import os

from svc.config import cfg


def load():
    value = cfg.get(
        "plugins.p04.interval",
        "on",
    )
    return locals()


def helper_20(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
