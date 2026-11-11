"""Prices and tax."""

TAX_RATE = 0.10


def subtotal(lines):
    """lines is a list of dicts with 'name', 'price' and 'qty'."""
    return sum(line["price"] * line["qty"] for line in lines)


def total(lines):
    """Subtotal plus tax, rounded to 2 decimals."""
    return round(subtotal(lines) * (1 + TAX_RATE), 2)
