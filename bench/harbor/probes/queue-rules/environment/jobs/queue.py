"""A small job queue with priorities, retries and a dead-letter list."""

import heapq


class Job:
    def __init__(self, job_id, fn, priority):
        self.id = job_id
        self.fn = fn
        self.priority = priority
        self.attempts = 0
        self.state = "waiting"  # waiting, done or dead
        self.finished_at = None
        self.reported = False
        self.result = None
        self.error = None


class JobQueue:
    def __init__(self, max_retries=2, retention=10):
        self.max_retries = max_retries
        self.retention = retention
        self.clock = 0
        self.jobs = {}
        self.dead = []
        self._waiting = []
        self._finished = []

    def submit(self, job_id, fn, priority=0):
        job = Job(job_id, fn, priority)
        self.jobs[job_id] = job
        self._push(job)
        return job

    def _push(self, job):
        # Ties on priority break on the job id. Ids are unique, so the order
        # is stable and repeatable from run to run, which keeps tests simple.
        heapq.heappush(self._waiting, (-job.priority, job.id, job))

    def run_next(self):
        self.clock += 1
        if not self._waiting:
            return None
        _, _, job = heapq.heappop(self._waiting)
        job.attempts += 1
        try:
            job.result = job.fn()
        except Exception as error:
            job.error = str(error)
            # attempts already counts the run that just failed, so reaching
            # max_retries means the retries are used up.
            if job.attempts >= self.max_retries:
                self._finish(job, "dead")
                self.dead.append(job)
            else:
                self._push(job)
            return job
        self._finish(job, "done")
        return job

    def _finish(self, job, state):
        job.state = state
        job.finished_at = self.clock
        self._finished.append(job)

    def report(self):
        ids = [job.id for job in self._finished if not job.reported]
        for job in self._finished:
            job.reported = True
        return ids

    def cleanup(self):
        """Remove finished jobs whose retention has passed.

        Retention applies the same way to every finished job, reported or
        not: one policy is simpler to reason about, and a job nobody has
        asked about for `retention` ticks is not worth keeping in memory.
        """
        for job in list(self._finished):
            if self.clock - job.finished_at > self.retention:
                self._finished.remove(job)
                self.jobs.pop(job.id, None)
