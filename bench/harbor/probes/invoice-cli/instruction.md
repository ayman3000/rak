Write /app/invoice.py, a command-line tool that prints an invoice from a CSV file. Run it as `python3 /app/invoice.py FILE [--discount PCT] [--currency CODE]`. /app/data/items.csv is an example input. Only the Python standard library is available.

Input: a CSV file with the header `name,qty,price`. Values may have spaces around them; a name may contain commas when it is quoted.

Output, on standard output:
1. One line per item: `f"{name:<20} {qty:>5} {price:>12} {total:>12}"`, where price and total have two decimals and a comma every three digits (`1,234.50`), and total is qty × price.
2. A name longer than 20 characters is cut to its first 19 characters followed by `~`.
3. Items with qty 0 are left out.
4. Items are sorted by total, largest first; equal totals by name, A to Z.
5. Then a line of 51 `-` characters, then `f"{'TOTAL':<20} {'':>5} {'':>12} {grand:>12}"`.
6. With `--discount PCT` (a number from 0 to 100), a line `f"{'DISCOUNT ' + PCT + '%':<20} {'':>5} {'':>12} {-amount:>12}"` comes between the dashes and TOTAL, where PCT is written as given, and TOTAL is the total after the discount.
7. The last line is `Currency: CODE`; CODE is USD when `--currency` is not given.
8. A line whose name matches an earlier line's name, ignoring upper/lower case and surrounding spaces, is merged into it: the quantities are added and the first spelling is kept. If the two prices differ, that is an error (below). An item whose merged qty is 0 is left out.
9. `--discount 0` prints no DISCOUNT line.
10. The file may start with a UTF-8 byte-order mark.
11. Money is rounded to cents half up (2.675 becomes 2.68, 2.665 becomes 2.67): every amount shown, prices included, and each line total and the discount amount before they are added up.

Errors, on standard error:
- A negative qty or price: `error: line N: negative value`, where N is the line number in the file (the header is line 1), and exit code 2. Nothing is printed on standard output.
- A line repeating an earlier name with a different price: `error: line N: price conflict for NAME`, where NAME is the first spelling, and exit code 2.
- A file that does not exist: `error: no such file: FILE` (FILE as given), and exit code 1.
