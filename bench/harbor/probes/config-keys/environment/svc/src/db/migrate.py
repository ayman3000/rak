import os

from svc.config import cfg


def load():
    url = cfg.require("db.url")
    dry = os.getenv("SVC_MIGRATE_DRY")
    return locals()


def helper_3(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
