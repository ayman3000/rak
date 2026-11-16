import os

from checklib import check, finish

N = 4
GOAL = tuple(list(range(1, N * N)) + [0])
BEST = 54  # the optimal length, found with IDA* (solution/solve.sh)
DIRS = {"U": -N, "D": N, "L": -1, "R": 1}

start = tuple(int(x) for x in open("/app/puzzle.txt").read().split())
moves = open("/app/moves.txt").read().strip() if os.path.exists("/app/moves.txt") else None


def step(state, move):
    blank = state.index(0)
    r, c = divmod(blank, N)
    if (move == "U" and r == 0) or (move == "D" and r == N - 1) or (move == "L" and c == 0) or (move == "R" and c == N - 1):
        return None
    other = blank + DIRS[move]
    cells = list(state)
    cells[blank], cells[other] = cells[other], cells[blank]
    return tuple(cells)


def valid():
    assert moves and set(moves) <= set("UDLR"), repr(moves)
    state = start
    for move in moves:
        state = step(state, move)
        assert state is not None, f"illegal move {move}"
    assert state == GOAL, "does not reach the goal"
    return True


check("moves.txt solves the puzzle", valid)
check(f"shortest ({BEST} moves)", lambda: moves is not None and len(moves) == BEST)
finish()
