The service in /app/svc reads its configuration in many places. Write /app/KEYS.md: a Markdown table with the header row `| key | default | files |` (then the usual `|---|---|---|` row) listing every configuration key the code under /app/svc/src reads, one row per key, sorted by key in plain byte order (uppercase before lowercase).

- Keys are read with `cfg.get(...)`, `cfg.get_int(...)`, `cfg.require(...)`, and from the environment with `os.environ.get(...)` or `os.getenv(...)`.
- default: the default exactly as written in the code, without its quotes; an empty string is written `""`; `(none)` when there is no default.
- files: the path of every file that reads the key, relative to /app/svc/src, sorted, separated by `, `.
- Only Python code counts: not comments, not commented-out code, not other files.
