import unittest
import textstats


class Hidden(unittest.TestCase):
    def test_basic_order(self):
        text = "the cat and the hat and the bat"
        self.assertEqual(textstats.top_words(text), [("the", 3), ("and", 2), ("bat", 1)])

    def test_case_and_punctuation(self):
        text = "Dog! dog? DOG... cat,cat;bird"
        self.assertEqual(textstats.top_words(text, 2), [("dog", 3), ("cat", 2)])

    def test_apostrophes_stay_in_words(self):
        self.assertEqual(textstats.top_words("don't stop, don't", 1), [("don't", 2)])

    def test_digits_separate_words(self):
        self.assertEqual(textstats.top_words("ab1ab2cd", 5), [("ab", 2), ("cd", 1)])

    def test_ties_are_alphabetical(self):
        self.assertEqual(textstats.top_words("pear apple fig", 3), [("apple", 1), ("fig", 1), ("pear", 1)])

    def test_edges(self):
        self.assertEqual(textstats.top_words("", 3), [])
        self.assertEqual(textstats.top_words("123 !!!", 3), [])
        self.assertEqual(textstats.top_words("a b c", 0), [])
        self.assertEqual(textstats.top_words("a a b", 10), [("a", 2), ("b", 1)])

    def test_returns_tuples_in_a_list(self):
        result = textstats.top_words("x y x")
        self.assertIsInstance(result, list)
        self.assertIsInstance(result[0], tuple)

    def test_old_functions_still_work(self):
        self.assertEqual(textstats.word_count("one two  three"), 3)
        self.assertEqual(textstats.line_count("a\nb\n"), 2)
