import os
import subprocess
import sys

from checklib import check, finish

CASES = "/tmp/invoice-cases"
os.makedirs(CASES, exist_ok=True)


def case(name, text):
    path = os.path.join(CASES, name)
    with open(path, "w") as f:
        f.write(text)
    return path


def run(tool, *args):
    done = subprocess.run([sys.executable, tool, *args], capture_output=True, text=True, timeout=30)
    return done.returncode, done.stdout, done.stderr


def same(*args):
    want = run("/tests/reference.py", *args)
    got = run("/app/invoice.py", *args)
    assert got[0] == want[0], f"exit {got[0]}, wanted {want[0]}; stderr {got[2]!r}"
    assert got[1].rstrip("\n") == want[1].rstrip("\n"), f"stdout {got[1]!r}\nwanted {want[1]!r}"
    if want[2]:
        assert got[2].strip() == want[2].strip(), f"stderr {got[2]!r}, wanted {want[2]!r}"
    return True


basic = case("basic.csv", "name,qty,price\nWidget,3,4.50\nGadget,1,12.00\nBolt,100,0.25\n")
spaces = case("spaces.csv", "name,qty,price\n  Widget , 2 , 1.10 \n\"Nuts, assorted\",4,2.00\n")
long = case("long.csv", "name,qty,price\nAn extremely long product name,1,1.00\nExactly twenty chars,1,2.00\n")
zero = case("zero.csv", "name,qty,price\nKept,1,5.00\nDropped,0,99.00\n")
ties = case("ties.csv", "name,qty,price\nZeta,2,5.00\nAlpha,1,10.00\nMid,1,20.00\n")
big = case("big.csv", "name,qty,price\nServer,3,12345.67\nRack,1,999.99\n")
rounding = case("round.csv", "name,qty,price\nHalfA,1,2.675\nHalfB,1,2.665\nThird,3,0.335\n")
conflict = case("conflict.csv", "name,qty,price\nWidget,2,4.50\nGadget,1,1.00\nWIDGET,1,4.75\n")
bom = case("bom.csv", "\ufeffname,qty,price\nWidget,1,2.00\n")
negative = case("neg.csv", "name,qty,price\nGood,1,1.00\nBad,-2,1.00\n")
negprice = case("negprice.csv", "name,qty,price\nGood,1,1.00\nAlso good,2,3.00\nBad,1,-0.50\n")

check("example file", lambda: same("/app/data/items.csv"))
check("basic lines and total", lambda: same(basic))
check("spaces and quoted commas", lambda: same(spaces))
check("long names cut with ~", lambda: same(long))
check("qty 0 left out", lambda: same(zero))
check("sorted by total, ties by name", lambda: same(ties))
check("thousands separators", lambda: same(big))
check("rounding half up", lambda: same(rounding))
check("discount line", lambda: same(big, "--discount", "12.5"))
check("currency", lambda: same(basic, "--currency", "EUR"))
check("discount and currency", lambda: same(rounding, "--currency", "GBP", "--discount", "10"))
check("negative qty", lambda: same(negative))
check("negative price, line number", lambda: same(negprice))
check("missing file", lambda: same("/app/data/nope.csv"))
check("same names merged", lambda: same(case("merge2.csv", "name,qty,price\nWidget,2,4.50\nBolt,1,0.25\n  widget ,3,4.50\n")))
check("price conflict", lambda: same(conflict))
check("byte-order mark", lambda: same(bom))
check("discount 0: no line", lambda: same(basic, "--discount", "0"))
check("merge then sort", lambda: same(case("merge3.csv", "name,qty,price\nA,1,10.00\nB,3,4.00\na,1,10.00\n")))
finish()
