#!/bin/bash
cat > /tmp/solver.py <<'PYEOF'
"""Breadth-first search for the shortest solution: the checker uses its
length, and solution/solve.sh runs it."""

from collections import deque

GOAL = (1, 2, 3, 4, 5, 6, 7, 8, 0)
MOVES = {"U": -3, "D": 3, "L": -1, "R": 1}


def step(state, move):
    blank = state.index(0)
    row, col = divmod(blank, 3)
    if (move == "U" and row == 0) or (move == "D" and row == 2) or (move == "L" and col == 0) or (move == "R" and col == 2):
        return None
    other = blank + MOVES[move]
    cells = list(state)
    cells[blank], cells[other] = cells[other], cells[blank]
    return tuple(cells)


def solve(start):
    seen = {start: ""}
    queue = deque([start])
    while queue:
        state = queue.popleft()
        if state == GOAL:
            return seen[state]
        for move in "UDLR":
            nxt = step(state, move)
            if nxt is not None and nxt not in seen:
                seen[nxt] = seen[state] + move
                queue.append(nxt)
    return None


def read(path):
    return tuple(int(x) for x in open(path).read().split())


if __name__ == "__main__":
    print(solve(read("/app/puzzle.txt")))
PYEOF
python3 /tmp/solver.py > /app/moves.txt
