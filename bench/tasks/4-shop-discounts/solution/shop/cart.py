from .pricing import total, normalize


class Cart:
    def __init__(self):
        self.lines = []
        self.code = None

    def add(self, name, price, qty=1):
        for line in self.lines:
            if line["name"] == name:
                line["qty"] += qty
                return
        self.lines.append({"name": name, "price": price, "qty": qty})

    def apply_code(self, code):
        self.code = normalize(code)

    def total(self):
        return total(self.lines, self.code)
