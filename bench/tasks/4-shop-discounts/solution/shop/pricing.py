TAX_RATE = 0.10
CODES = {"SAVE10", "FLAT5", "FREESHIP"}


def normalize(code):
    if code is None:
        return None
    upper = str(code).upper()
    if upper not in CODES:
        raise ValueError(f"unknown code: {code}")
    return upper


def subtotal(lines):
    return sum(line["price"] * line["qty"] for line in lines)


def total(lines, code=None):
    code = normalize(code)
    s = subtotal(lines)
    if code == "SAVE10":
        s *= 0.9
    elif code == "FLAT5":
        s = max(0, s - 5)
    return round(s * (1 + TAX_RATE), 2)
