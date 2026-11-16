import os

from svc.config import cfg


def load():
    PLUGINS_P11_LIMIT = "plugins.p11.limit"
    value = cfg.get_int(PLUGINS_P11_LIMIT, 303)
    return locals()


def helper_27(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
