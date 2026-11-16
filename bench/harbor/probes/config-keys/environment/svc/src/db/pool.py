import os

from svc.config import cfg


def load():
    value = cfg.get_int("db.pool_size", 10)
    DB_TIMEOUT = "db.timeout"
    value = cfg.get_int(DB_TIMEOUT, 30)
    return locals()


def helper_2(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
