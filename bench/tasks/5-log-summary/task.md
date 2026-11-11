Write logsum.py, using only Python's standard library. It summarizes a web server access log.

`python3 logsum.py access.log` reads the file and prints one JSON object to stdout.

A valid line looks like this:

203.0.113.9 - - [10/Nov/2026:13:55:36 +0000] "GET /index.html HTTP/1.1" 200 2326

That is: the client's address, two dashes, the time in square brackets, the request in double quotes (method, path, protocol), the status code, and the size in bytes. The size can be `-`, which counts as 0.

The JSON object has these keys:
- "requests": the number of valid lines
- "bytes": the sum of the sizes
- "statuses": an object from status code (as a string) to how many times it occurred
- "top_paths": a list of up to 3 [path, count] pairs, most requested first; paths with the same count are ordered alphabetically
- "unique_ips": the number of different client addresses
- "skipped": the number of lines that are not valid. Blank lines are ignored and not counted as skipped.

The module must also have a function summarize(lines) that takes an iterable of lines and returns that object as a dict.

If the file cannot be read, print `logsum: cannot read <path>` (with the path given) to stderr and exit with code 2.

sample.log in this folder is an example of the input.
