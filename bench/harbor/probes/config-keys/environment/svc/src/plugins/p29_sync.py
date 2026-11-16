import os

from svc.config import cfg


def load():
    PLUGINS_P29_LIMIT = "plugins.p29.limit"
    value = cfg.get_int(PLUGINS_P29_LIMIT, 409)
    return locals()


def helper_45(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
