import unittest
from inventory import total_value, low_stock, restock


def stock():
    return [
        {"name": "pen", "price": 1.5, "qty": 10},
        {"name": "ink", "price": 4.0, "qty": 5},
        {"name": "book", "price": 12.25, "qty": 2},
    ]


class InventoryTests(unittest.TestCase):
    def test_total_value_counts_quantity(self):
        self.assertEqual(total_value(stock()), 59.5)

    def test_low_stock_includes_the_threshold(self):
        self.assertEqual(low_stock(stock()), ["book", "ink"])

    def test_restock_adds(self):
        items = stock()
        self.assertEqual(restock(items, "book", 3), 5)


if __name__ == "__main__":
    unittest.main()
