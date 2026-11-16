import os

from svc.config import cfg


def load():
    value = os.environ["SVC_SECRET_KEY"]
    return locals()


def helper_16(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
