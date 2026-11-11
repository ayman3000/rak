def total_value(items):
    return round(sum(i["price"] * i["qty"] for i in items), 2)


def low_stock(items, threshold=5):
    return sorted(i["name"] for i in items if i["qty"] <= threshold)


def restock(items, name, amount):
    for item in items:
        if item["name"] == name:
            item["qty"] += amount
            return item["qty"]
    raise KeyError(name)
