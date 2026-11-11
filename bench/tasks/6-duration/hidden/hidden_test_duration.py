import unittest
import duration


class Hidden(unittest.TestCase):
    def test_parse_valid(self):
        cases = {"1h30m": 5400, "2d": 172800, "1d2h3m4s": 93784, "1h 30m": 5400, "90": 90, "0": 0,
                 "0s": 0, "45s": 45, "10m": 600, "1d 1s": 86401, "100h": 360000}
        for text, seconds in cases.items():
            with self.subTest(text=text):
                self.assertEqual(duration.parse(text), seconds)

    def test_parse_invalid(self):
        for text in ["", "   ", "abc", "1x", "30m1h", "1h2h", "-5", "-1h", "1h30", "h", "1.5h", "1h,30m", "5 5"]:
            with self.subTest(text=text):
                with self.assertRaises(ValueError):
                    duration.parse(text)

    def test_format(self):
        cases = {5400: "1h30m", 3600: "1h", 86461: "1d1m1s", 0: "0s", 172800: "2d", 59: "59s", 93784: "1d2h3m4s"}
        for seconds, text in cases.items():
            with self.subTest(seconds=seconds):
                self.assertEqual(duration.format(seconds), text)

    def test_format_negative(self):
        with self.assertRaises(ValueError):
            duration.format(-1)

    def test_round_trip(self):
        for seconds in [1, 60, 61, 3599, 86399, 86400, 1_000_000]:
            self.assertEqual(duration.parse(duration.format(seconds)), seconds)
