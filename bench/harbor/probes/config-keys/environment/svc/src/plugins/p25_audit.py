import os

from svc.config import cfg


def load():
    value = cfg.get_int("plugins.p25.enabled", 160)
    return locals()


def helper_41(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
