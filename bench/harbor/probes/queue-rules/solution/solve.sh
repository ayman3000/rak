#!/bin/bash
# The three fixes: ties break on a submit counter, a job dies after
# max_retries + 1 runs, and cleanup keeps jobs that were not reported.
set -e
cd /app/jobs
python3 - <<'PY'
src = open("queue.py").read()
src = src.replace("import heapq\n", "import heapq\nimport itertools\n", 1)
src = src.replace("        self._finished = []\n", "        self._finished = []\n        self._order = itertools.count()\n", 1)
src = src.replace("heapq.heappush(self._waiting, (-job.priority, job.id, job))",
                  "heapq.heappush(self._waiting, (-job.priority, next(self._order), job))", 1)
src = src.replace("if job.attempts >= self.max_retries:", "if job.attempts > self.max_retries:", 1)
src = src.replace("if self.clock - job.finished_at > self.retention:",
                  "if job.reported and self.clock - job.finished_at > self.retention:", 1)
open("queue.py", "w").write(src)
PY
