import os

from svc.config import cfg


def load():
    host = cfg.get("mail.host", "smtp.local")
    sender = cfg.require("mail.from")
    return locals()


def helper_11(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
