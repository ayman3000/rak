/app/puzzle.txt holds a 4x4 sliding puzzle: the numbers 1 to 15 and a 0 for the empty square, one row per line. A move slides the empty square up (U), down (D), left (L) or right (R), swapping it with the neighbour on that side.

Write the shortest sequence of moves that reaches

```
 1  2  3  4
 5  6  7  8
 9 10 11 12
13 14 15  0
```

to /app/moves.txt, as one line of letters with nothing else (for example `RDLU`). It must be a shortest solution: no solution with fewer moves exists.
