import os

from svc.config import cfg


def load():
    value = cfg.get("features", "")
    # value = cfg.get("features.legacy", "old")
    return locals()


def helper_14(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
