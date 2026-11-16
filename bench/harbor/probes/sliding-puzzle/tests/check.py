from checklib import check, finish
from solver import GOAL, read, solve, step

start = read("/app/puzzle.txt")
best = len(solve(start))
moves = open("/app/moves.txt").read().strip() if __import__("os").path.exists("/app/moves.txt") else None


def valid():
    assert moves is not None and moves and set(moves) <= set("UDLR"), repr(moves)
    state = start
    for move in moves:
        state = step(state, move)
        assert state is not None, f"illegal move {move}"
    assert state == GOAL, "does not reach the goal"
    return True


check("moves.txt solves the puzzle", valid)
check(f"shortest ({best} moves)", lambda: moves is not None and len(moves) == best)
finish()
