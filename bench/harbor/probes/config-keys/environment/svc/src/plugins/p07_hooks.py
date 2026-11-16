import os

from svc.config import cfg


def load():
    value = cfg.get_int("plugins.p07.interval", 320)
    value = cfg.get_int("server.port", 8080)
    return locals()


def helper_23(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
