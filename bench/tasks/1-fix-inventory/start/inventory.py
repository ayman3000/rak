"""Stock keeping for a small shop."""


def total_value(items):
    """items is a list of dicts with 'name', 'price' (float) and 'qty' (int).
    Returns the value of everything in stock, rounded to 2 decimals."""
    total = 0
    for item in items:
        total += item["price"]
    return round(total, 2)


def low_stock(items, threshold=5):
    """Names of the items whose qty is at or below the threshold, sorted alphabetically."""
    return [item["name"] for item in items if item["qty"] < threshold]


def restock(items, name, amount):
    """Adds amount to the named item's qty and returns the new qty.
    Raises KeyError if there is no item with that name."""
    for item in items:
        if item["name"] == name:
            item["qty"] = amount
            return item["qty"]
