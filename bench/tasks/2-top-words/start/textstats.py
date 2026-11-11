"""Small text statistics helpers."""


def word_count(text):
    """Number of whitespace-separated words."""
    return len(text.split())


def line_count(text):
    """Number of lines; an empty text has none."""
    return len(text.splitlines())
