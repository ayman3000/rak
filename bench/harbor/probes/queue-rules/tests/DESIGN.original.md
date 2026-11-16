# Job queue

`JobQueue` runs jobs one at a time. A job is a function with no arguments;
it succeeds when it returns and fails when it raises.

## Order

- `run_next()` runs the waiting job with the highest priority.
- Jobs with the same priority run in the order they were submitted.
- A job that is retried goes back in the queue behind the jobs of its
  priority that are already waiting.

## Retries

- A job that succeeds is done; it is never run again.
- A job that fails is retried at most `max_retries` times: it runs at most
  `max_retries + 1` times in all. When its last run fails it is dead and is
  added to the dead-letter list (`dead`).

## Time, reports and cleanup

- Each call to `run_next()` is one tick of the queue's clock, even when no
  job is waiting.
- `report()` returns the ids of the finished (done or dead) jobs that have
  not been reported yet, in the order they finished, and marks them reported.
  Every finished job is reported exactly once.
- `cleanup()` removes a finished job from `jobs` only when it has been
  reported and more than `retention` ticks have passed since it finished.
  A job that has not been reported is never removed.
