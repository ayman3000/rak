Write /app/stats.py: `python3 /app/stats.py FILE [options]` reads numbers from FILE, one per line, and prints statistics about them. /app/data/sample.txt is an example. Only the Python standard library is available.

Input: blank lines and lines starting with `#` are skipped. Any other line that is not a number is an error: print `error: line N: not a number` to standard error (N counts every line, from 1) and exit with code 3. A file with no numbers prints `no data` and exits with code 0.

Output, one `name: value` per line in this order: count, sum, mean, median, stdev, min, max, p90. Numbers are printed with 4 decimals, except count, which is a whole number. stdev is the sample standard deviation (n - 1), and 0.0000 for a single number. p90 is the 90th percentile with linear interpolation between closest ranks (as numpy's default does).

Options:
- `--json`: print one JSON object with the same names as keys instead of the lines (numbers not rounded, count an integer).
- `--bins N`: after the statistics, print a histogram of N equal-width bins from min to max, one line per bin: `[lo, hi) count` with lo and hi to 2 decimals; the last bin is `[lo, hi]` and includes max.
- `--trim P`: before computing anything, drop the lowest P% and highest P% of the numbers (rounding the count to drop down).
- `--precision D`: print D decimals instead of 4.
