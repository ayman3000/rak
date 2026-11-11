import re
from collections import Counter


def word_count(text):
    return len(text.split())


def line_count(text):
    return len(text.splitlines())


def top_words(text, n=3):
    counts = Counter(re.findall(r"[a-z']+", text.lower()))
    return sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))[:n]
