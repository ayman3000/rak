import os

from svc.config import cfg


def load():
    PLUGINS_P05_TARGET = "plugins.p05.target"
    value = cfg.get_int(PLUGINS_P05_TARGET, 73)
    return locals()


def helper_21(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
