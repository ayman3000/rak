import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

LOG = """\
10.0.0.1 - - [10/Nov/2026:10:00:00 +0000] "GET /a HTTP/1.1" 200 100
10.0.0.1 - - [10/Nov/2026:10:00:01 +0000] "GET /b HTTP/1.1" 200 50
10.0.0.2 - - [10/Nov/2026:10:00:02 +0000] "GET /a HTTP/1.1" 304 -
10.0.0.3 - - [10/Nov/2026:10:00:03 +0000] "POST /c HTTP/1.1" 500 7

garbage here
10.0.0.3 - - [10/Nov/2026:10:00:04 +0000] "GET /d HTTP/1.1" 200 1
10.0.0.3 - - [10/Nov/2026:10:00:05 +0000] "GET /b HTTP/1.1" 200 1
10.0.0.4 - - [10/Nov/2026:10:00:06 +0000] "GET /e HTTP/1.1" 404 2
10.0.0.9 [broken
"""

EXPECTED = {
    "requests": 7,
    "bytes": 161,
    "statuses": {"200": 4, "304": 1, "500": 1, "404": 1},
    "top_paths": [["/a", 2], ["/b", 2], ["/c", 1]],
    "unique_ips": 4,
    "skipped": 2,
}


def run(*args):
    return subprocess.run([sys.executable, os.path.join(HERE, "logsum.py"), *args],
                          capture_output=True, text=True, timeout=20)


class Hidden(unittest.TestCase):
    def test_command_line(self):
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as f:
            f.write(LOG)
        r = run(f.name)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout), EXPECTED)

    def test_summarize_function(self):
        import logsum
        result = logsum.summarize(LOG.splitlines())
        self.assertEqual(json.loads(json.dumps(result)), EXPECTED)

    def test_empty_input(self):
        import logsum
        result = json.loads(json.dumps(logsum.summarize([])))
        self.assertEqual(result, {"requests": 0, "bytes": 0, "statuses": {}, "top_paths": [], "unique_ips": 0, "skipped": 0})

    def test_lines_may_end_with_newlines(self):
        import logsum
        result = logsum.summarize(LOG.splitlines(keepends=True))
        self.assertEqual(result["requests"], 7)
        self.assertEqual(result["skipped"], 2)

    def test_missing_file(self):
        r = run("/no/such/file.log")
        self.assertEqual(r.returncode, 2)
        self.assertEqual(r.stderr.strip(), "logsum: cannot read /no/such/file.log")
        self.assertEqual(r.stdout.strip(), "")
