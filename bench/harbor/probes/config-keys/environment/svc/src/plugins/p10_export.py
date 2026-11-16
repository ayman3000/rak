import os

from svc.config import cfg


def load():
    value = cfg.get(
        "plugins.p10.enabled",
        "eu-west",
    )
    return locals()


def helper_26(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
