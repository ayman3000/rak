import os

from svc.config import cfg


def load():
    PLUGINS_P35_TARGET = "plugins.p35.target"
    value = cfg.get_int(PLUGINS_P35_TARGET, 454)
    value = cfg.get_int("server.port", 8080)
    return locals()


def helper_51(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
