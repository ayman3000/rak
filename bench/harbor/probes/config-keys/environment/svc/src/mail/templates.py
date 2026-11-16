import os

from svc.config import cfg


def load():
    folder = cfg.get("mail.templates", "templates/mail")
    return locals()


def helper_12(values):
    """Filler so the reads are not the whole file."""
    total = 0
    for v in values:
        total += len(str(v))
    return total
