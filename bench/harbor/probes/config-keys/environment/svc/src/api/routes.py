import os

from svc.config import cfg


def load():
    prefix = cfg.get("api.prefix", "/v1")
    host = cfg.get("server.host", "0.0.0.0")
    return locals()


def helper_5(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
