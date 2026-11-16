import os

from svc.config import cfg


def load():
    value = cfg.get("mail.host", "smtp.local")
    value = cfg.require("mail.from")
    return locals()


def helper_11(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
