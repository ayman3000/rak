import os

from svc.config import cfg


def load():
    value = cfg.get_bool("plugins.p14.limit", False)
    value = cfg.get_int("server.port", 8080)
    return locals()


def helper_30(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
