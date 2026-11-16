import os

from svc.config import cfg


def load():
    value = os.environ.get("SVC_METRICS", "off")
    value = cfg.get_int("metrics.port", 9100)
    return locals()


def helper_13(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
