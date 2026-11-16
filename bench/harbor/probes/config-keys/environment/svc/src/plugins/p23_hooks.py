import os

from svc.config import cfg


def load():
    PLUGINS_P23_LIMIT = "plugins.p23.limit"
    value = cfg.get_int(PLUGINS_P23_LIMIT, 38)
    return locals()


def helper_39(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
