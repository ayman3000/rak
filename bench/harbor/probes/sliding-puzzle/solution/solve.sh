#!/bin/bash
cat > /tmp/ida.py <<'PYEOF'
"""IDA* with Manhattan distance + linear conflict for the 15-puzzle: finds an
instance whose optimal solution is long, for the sliding-puzzle probe."""

import random
import sys
import time

N = 4
GOAL = tuple(list(range(1, N * N)) + [0])
POS = {v: divmod(i, N) for i, v in enumerate(GOAL)}
DIRS = {"U": -N, "D": N, "L": -1, "R": 1}
OPP = {"U": "D", "D": "U", "L": "R", "R": "L"}


def neighbours(state, blank):
    r, c = divmod(blank, N)
    for m, d in DIRS.items():
        if (m == "U" and r == 0) or (m == "D" and r == N - 1) or (m == "L" and c == 0) or (m == "R" and c == N - 1):
            continue
        o = blank + d
        s = list(state)
        s[blank], s[o] = s[o], s[blank]
        yield m, tuple(s), o


def h(state):
    dist = 0
    for i, v in enumerate(state):
        if v:
            r, c = divmod(i, N)
            gr, gc = POS[v]
            dist += abs(r - gr) + abs(c - gc)
    # linear conflict
    for r in range(N):
        row = [state[r * N + c] for c in range(N)]
        tiles = [v for v in row if v and POS[v][0] == r]
        for i in range(len(tiles)):
            for j in range(i + 1, len(tiles)):
                if POS[tiles[i]][1] > POS[tiles[j]][1]:
                    dist += 2
    for c in range(N):
        col = [state[r * N + c] for r in range(N)]
        tiles = [v for v in col if v and POS[v][1] == c]
        for i in range(len(tiles)):
            for j in range(i + 1, len(tiles)):
                if POS[tiles[i]][0] > POS[tiles[j]][0]:
                    dist += 2
    return dist


def ida(start):
    bound = h(start)
    path = []

    def search(state, blank, g, bound, last):
        f = g + h(state)
        if f > bound:
            return f
        if state == GOAL:
            return True
        best = float("inf")
        for m, nxt, nb in neighbours(state, blank):
            if last and m == OPP[last]:
                continue
            path.append(m)
            t = search(nxt, nb, g + 1, bound, m)
            if t is True:
                return True
            best = min(best, t)
            path.pop()
        return best

    while True:
        t = search(start, start.index(0), 0, bound, None)
        if t is True:
            return "".join(path)
        bound = t


def scramble(steps, seed):
    rng = random.Random(seed)
    s, b, last = GOAL, GOAL.index(0), None
    for _ in range(steps):
        opts = [(m, n, nb) for m, n, nb in neighbours(s, b) if not last or m != OPP[last]]
        m, s, b = rng.choice(opts)
        last = m
    return s


if __name__ == "__main__":
    start = tuple(int(x) for x in open("/app/puzzle.txt").read().split())
    print(ida(start))
PYEOF
python3 /tmp/ida.py > /app/moves.txt
