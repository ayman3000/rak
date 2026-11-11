import unittest
from shop import Cart
from shop import pricing


def cart():
    c = Cart()
    c.add("mug", 8.00, 2)      # 16.00
    c.add("tea", 4.00)         #  4.00  -> subtotal 20.00
    return c


class Hidden(unittest.TestCase):
    def test_no_code_is_unchanged(self):
        self.assertEqual(cart().total(), 22.0)
        self.assertEqual(pricing.total([{"name": "a", "price": 10, "qty": 1}]), 11.0)

    def test_save10(self):
        c = cart()
        c.apply_code("SAVE10")
        self.assertEqual(c.total(), 19.8)

    def test_flat5(self):
        c = cart()
        c.apply_code("FLAT5")
        self.assertEqual(c.total(), 16.5)

    def test_flat5_never_below_zero(self):
        c = Cart()
        c.add("gum", 1.50, 2)
        c.apply_code("FLAT5")
        self.assertEqual(c.total(), 0)

    def test_freeship_changes_nothing(self):
        c = cart()
        c.apply_code("FREESHIP")
        self.assertEqual(c.total(), 22.0)

    def test_codes_ignore_case(self):
        c = cart()
        c.apply_code("save10")
        self.assertEqual(c.total(), 19.8)

    def test_later_code_replaces_earlier(self):
        c = cart()
        c.apply_code("SAVE10")
        c.apply_code("FLAT5")
        self.assertEqual(c.total(), 16.5)

    def test_unknown_code_raises_and_keeps_the_old_one(self):
        c = cart()
        c.apply_code("SAVE10")
        with self.assertRaises(ValueError):
            c.apply_code("BOGUS")
        self.assertEqual(c.total(), 19.8)

    def test_pricing_total_takes_the_code(self):
        lines = [{"name": "a", "price": 20.0, "qty": 1}]
        self.assertEqual(pricing.total(lines, "SAVE10"), 19.8)
        self.assertEqual(pricing.total(lines, code="flat5"), 16.5)
        self.assertEqual(pricing.total(lines, None), 22.0)
        with self.assertRaises(ValueError):
            pricing.total(lines, "NOPE")

    def test_same_name_merges(self):
        c = Cart()
        c.add("mug", 8.00)
        c.add("mug", 9.99, 2)
        self.assertEqual(len(c.lines), 1)
        self.assertEqual(c.lines[0]["qty"], 3)
        self.assertEqual(c.lines[0]["price"], 8.00)
        self.assertEqual(c.total(), 26.4)

    def test_rounding(self):
        c = Cart()
        c.add("x", 0.99, 3)
        c.apply_code("SAVE10")
        self.assertEqual(c.total(), 2.94)
