import os

from svc.config import cfg


def load():
    value = cfg["api.token"]
    value = cfg.get_int("api.token_ttl", 3600)
    return locals()


def helper_4(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
