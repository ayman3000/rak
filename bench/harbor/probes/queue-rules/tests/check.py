import sys

sys.path.insert(0, "/app")

from checklib import check, finish  # noqa: E402


def queue(**kwargs):
    from jobs.queue import JobQueue

    return JobQueue(**kwargs)


def ok():
    return "ok"


def failing():
    raise RuntimeError("boom")


def fails_once():
    state = {"runs": 0}

    def fn():
        state["runs"] += 1
        if state["runs"] == 1:
            raise RuntimeError("first run fails")
        return "ok"

    return fn


def ticks(q, n):
    for _ in range(n):
        q.run_next()


def run_order(q):
    order = []
    while True:
        job = q.run_next()
        if job is None:
            return order
        order.append(job.id)


def design_unchanged():
    return open("/app/jobs/DESIGN.md").read() == open("/tests/DESIGN.original.md").read()


def unreported_survives_cleanup():
    q = queue(retention=2)
    q.submit("a", ok)
    q.run_next()
    ticks(q, 5)
    q.cleanup()
    assert "a" in q.jobs, "an unreported job was removed by cleanup"
    assert q.report() == ["a"], "the job never showed up in the report"
    return True


def reported_is_cleaned():
    q = queue(retention=2)
    q.submit("a", ok)
    q.run_next()
    assert q.report() == ["a"]
    ticks(q, 5)
    q.cleanup()
    assert "a" not in q.jobs, "a reported job past its retention was kept"
    assert q.report() == [], "a job was reported twice"
    return True


def retries_are_max_retries():
    for max_retries in (0, 1, 2, 3):
        q = queue(max_retries=max_retries)
        job = q.submit("bad", failing)
        run_order(q)
        assert job.attempts == max_retries + 1, f"max_retries={max_retries}: ran {job.attempts} times"
        assert job.state == "dead" and q.dead == [job]
    return True


def success_runs_once():
    q = queue(max_retries=3)
    job = q.submit("good", ok)
    run_order(q)
    assert job.attempts == 1 and job.state == "done" and q.dead == []
    return True


def same_priority_in_submit_order():
    q = queue()
    for job_id in ["m", "b", "z", "a"]:
        q.submit(job_id, ok)
    q.submit("urgent", ok, priority=5)
    order = run_order(q)
    assert order == ["urgent", "m", "b", "z", "a"], order
    return True


def retry_goes_behind_waiting_jobs():
    q = queue(max_retries=2)
    q.submit("a", fails_once())
    q.submit("b", ok)
    q.submit("c", ok)
    order = run_order(q)
    assert order == ["a", "b", "c", "a"], order
    return True


def report_in_finish_order():
    q = queue(max_retries=1)
    q.submit("x", fails_once())
    q.submit("y", ok)
    run_order(q)
    assert q.report() == ["y", "x"]
    return True


check("DESIGN.md unchanged", design_unchanged)
check("unreported job survives cleanup", unreported_survives_cleanup)
check("reported job is cleaned up", reported_is_cleaned)
check("a failing job runs max_retries + 1 times", retries_are_max_retries)
check("a job that succeeds runs once", success_runs_once)
check("same priority runs in submit order", same_priority_in_submit_order)
check("a retry goes behind the waiting jobs", retry_goes_behind_waiting_jobs)
check("report in the order jobs finished", report_in_finish_order)
finish()
