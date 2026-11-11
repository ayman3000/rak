Add a function top_words(text, n=3) to textstats.py.

It returns the n most frequent words in the text as a list of (word, count) tuples, most frequent first.
- Words are compared without regard to case and returned in lower case.
- A word is a run of the letters a–z and apostrophes ('). Every other character separates words.
- Words with the same count are ordered alphabetically.
- If n is 0 or the text has no words, it returns an empty list. If there are fewer than n different words, it returns them all.

Leave the existing functions working as they are.
