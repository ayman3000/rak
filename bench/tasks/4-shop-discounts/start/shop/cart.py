"""The shopping cart."""

from .pricing import total


class Cart:
    def __init__(self):
        self.lines = []

    def add(self, name, price, qty=1):
        self.lines.append({"name": name, "price": price, "qty": qty})

    def total(self):
        return total(self.lines)
