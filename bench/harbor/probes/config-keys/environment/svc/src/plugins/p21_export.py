import os

from svc.config import cfg


def load():
    value = cfg.require("plugins.p21.interval")
    value = cfg.get_int("server.port", 8080)
    return locals()


def helper_37(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
