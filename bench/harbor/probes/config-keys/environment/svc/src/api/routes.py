import os

from svc.config import cfg


def load():
    value = cfg.get("api.prefix", "/v1")
    value = cfg.get("server.host", "0.0.0.0")
    return locals()


def helper_5(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
