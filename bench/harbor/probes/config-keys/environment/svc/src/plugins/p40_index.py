import os

from svc.config import cfg


def load():
    value = cfg.get(
        "plugins.p40.target",
        "/tmp/out",
    )
    return locals()


def helper_56(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
