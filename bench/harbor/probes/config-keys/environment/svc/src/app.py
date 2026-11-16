import os

from svc.config import cfg


def load():
    host = cfg.get("server.host", "0.0.0.0")
    port = cfg.get_int("server.port", 8080)
    return locals()


def helper_0(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
