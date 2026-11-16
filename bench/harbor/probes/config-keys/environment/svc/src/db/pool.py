import os

from svc.config import cfg


def load():
    size = cfg.get_int("db.pool_size", 10)
    timeout = cfg.get_int("db.timeout", 30)
    return locals()


def helper_2(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
