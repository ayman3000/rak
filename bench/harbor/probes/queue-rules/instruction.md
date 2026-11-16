The job queue in /app/jobs/ is not working correctly. Users report:

- Some finished jobs never show up in the report.
- Failing jobs end up in the dead-letter list sooner than they should.
- Jobs with the same priority sometimes run in the wrong order.

How the queue should behave is described in /app/jobs/DESIGN.md. Fix the queue. Do not change DESIGN.md, and keep the public classes, methods and their arguments as they are.
