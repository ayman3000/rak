import unittest
from inventory import total_value, low_stock, restock


def stock():
    return [
        {"name": "pen", "price": 1.5, "qty": 10},
        {"name": "ink", "price": 4.0, "qty": 5},
        {"name": "book", "price": 12.25, "qty": 2},
        {"name": "clip", "price": 0.1, "qty": 3},
    ]


class Hidden(unittest.TestCase):
    def test_total(self):
        self.assertEqual(total_value(stock()), 59.8)
        self.assertEqual(total_value([]), 0)

    def test_total_is_rounded(self):
        self.assertEqual(total_value([{"name": "a", "price": 0.1, "qty": 3}]), 0.3)

    def test_low_stock_sorted_and_inclusive(self):
        self.assertEqual(low_stock(stock()), ["book", "clip", "ink"])
        self.assertEqual(low_stock(stock(), threshold=2), ["book"])
        self.assertEqual(low_stock(stock(), threshold=1), [])

    def test_restock_adds_and_returns(self):
        items = stock()
        self.assertEqual(restock(items, "pen", 5), 15)
        self.assertEqual(items[0]["qty"], 15)

    def test_restock_unknown_raises(self):
        with self.assertRaises(KeyError):
            restock(stock(), "stapler", 1)
