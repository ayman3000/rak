import os

from svc.config import cfg


def load():
    rate = cfg.get_int("api.rate_limit", 100)
    # burst = cfg.get_int("api.burst", 20)
    return locals()


def helper_6(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
