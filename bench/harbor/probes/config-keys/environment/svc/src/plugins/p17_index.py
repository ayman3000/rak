import os

from svc.config import cfg


def load():
    PLUGINS_P17_TARGET = "plugins.p17.target"
    value = cfg.get_int(PLUGINS_P17_TARGET, 16)
    return locals()


def helper_33(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
