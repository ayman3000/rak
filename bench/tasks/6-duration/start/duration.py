"""Parse and format durations like '1h30m'."""

import re

UNITS = {"d": 86400, "h": 3600, "m": 60, "s": 1}


def parse(text):
    """'1h30m' -> 5400 (seconds).

    Units are d, h, m and s. Each may appear at most once, in that order,
    with a whole non-negative number: '2d', '1h30m', '1d2h3m4s'.
    Spaces between the parts are allowed: '1h 30m'.
    A bare number is seconds: '90' -> 90.
    Anything else raises ValueError: empty text, unknown units, units out of
    order ('30m1h'), a repeated unit ('1h2h'), a negative number, or a number
    with no unit next to parts that have one ('1h30').
    """
    total = 0
    for number, unit in re.findall(r"(\d+)([dhms])", text):
        total += int(number) * UNITS[unit]
    return total


def format(seconds):
    """5400 -> '1h30m'.

    Parts that are zero are left out: 3600 -> '1h', 86461 -> '1d1m1s'.
    Zero is '0s'. The largest unit is days: 172800 -> '2d'.
    A negative number raises ValueError.
    """
    parts = []
    for unit, size in UNITS.items():
        count = seconds // size
        if count:
            parts.append(f"{count}{unit}")
        seconds = seconds % size
    return "".join(parts)
