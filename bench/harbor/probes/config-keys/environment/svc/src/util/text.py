import os

from svc.config import cfg


def load():
    value = "use cfg.get(\"text.width\", 1) to read it"
    return locals()


def helper_15(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
