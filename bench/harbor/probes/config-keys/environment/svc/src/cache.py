import os

from svc.config import cfg


def load():
    ttl = cfg.get_int("cache.ttl", 300)
    url = os.getenv("SVC_CACHE_URL")
    return locals()


def helper_10(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
