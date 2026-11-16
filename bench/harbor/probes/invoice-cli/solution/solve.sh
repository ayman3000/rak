#!/bin/bash
cat > /app/invoice.py <<'PYEOF'
"""The reference invoice tool: the checker compares the agent's output with
this one's, and solution/solve.sh installs a copy."""

import csv
import sys
from decimal import ROUND_HALF_UP, Decimal

CENT = Decimal("0.01")


def money(value):
    return f"{value.quantize(CENT, rounding=ROUND_HALF_UP):,.2f}"


def main(argv):
    args = argv[1:]
    discount = None
    currency = "USD"
    path = None
    while args:
        arg = args.pop(0)
        if arg == "--discount":
            discount = args.pop(0)
        elif arg == "--currency":
            currency = args.pop(0)
        else:
            path = arg
    try:
        handle = open(path, newline="", encoding="utf-8-sig")
    except FileNotFoundError:
        print(f"error: no such file: {path}", file=sys.stderr)
        return 1
    merged = {}
    with handle:
        reader = csv.reader(handle)
        next(reader)
        for number, row in enumerate(reader, start=2):
            if not row:
                continue
            name, qty, price = (cell.strip() for cell in row)
            qty, price = int(qty), Decimal(price)
            if qty < 0 or price < 0:
                print(f"error: line {number}: negative value", file=sys.stderr)
                return 2
            key = name.lower()
            if key in merged:
                first, total_qty, first_price = merged[key]
                if price != first_price:
                    print(f"error: line {number}: price conflict for {first}", file=sys.stderr)
                    return 2
                merged[key] = (first, total_qty + qty, first_price)
            else:
                merged[key] = (name, qty, price)
    items = []
    for name, qty, price in merged.values():
        if qty == 0:
            continue
        total = (qty * price).quantize(CENT, rounding=ROUND_HALF_UP)
        items.append((name, qty, price, total))
    items.sort(key=lambda item: (-item[3], item[0]))
    lines = []
    for name, qty, price, total in items:
        shown = name if len(name) <= 20 else name[:19] + "~"
        lines.append(f"{shown:<20} {qty:>5} {money(price):>12} {money(total):>12}")
    grand = sum((item[3] for item in items), Decimal("0"))
    lines.append("-" * 51)
    if discount is not None and Decimal(discount) != 0:
        amount = (grand * Decimal(discount) / 100).quantize(CENT, rounding=ROUND_HALF_UP)
        grand -= amount
        lines.append(f"{'DISCOUNT ' + discount + '%':<20} {'':>5} {'':>12} {money(-amount):>12}")
    lines.append(f"{'TOTAL':<20} {'':>5} {'':>12} {money(grand):>12}")
    lines.append(f"Currency: {currency}")
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
PYEOF
