import re

UNITS = {"d": 86400, "h": 3600, "m": 60, "s": 1}
PATTERN = re.compile(r"^(?:(\d+)d)?\s*(?:(\d+)h)?\s*(?:(\d+)m)?\s*(?:(\d+)s)?$")


def parse(text):
    text = text.strip()
    if not text:
        raise ValueError("empty duration")
    if text.isdigit():
        return int(text)
    m = PATTERN.match(text)
    if not m:
        raise ValueError(f"bad duration: {text}")
    return sum(int(n) * size for n, size in zip(m.groups(), UNITS.values()) if n)


def format(seconds):
    if seconds < 0:
        raise ValueError("negative duration")
    if seconds == 0:
        return "0s"
    parts = []
    for unit, size in UNITS.items():
        count, seconds = divmod(seconds, size)
        if count:
            parts.append(f"{count}{unit}")
    return "".join(parts)
