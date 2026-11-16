import os

from svc.config import cfg


def load():
    flags = cfg.get("features", "")
    # old = cfg.get("features.legacy", "x")
    return locals()


def helper_14(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
