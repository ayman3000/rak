#!/bin/bash
cat > /app/KEYS.md <<'EOF'
| key | default | files |
|---|---|---|
| SVC_CACHE_URL | (none) | cache.py |
| SVC_LOG_LEVEL | info | log.py |
| SVC_METRICS | off | metrics.py |
| SVC_MIGRATE_DRY | (none) | db/migrate.py |
| SVC_TZ | UTC | jobs/schedule.py |
| api.prefix | /v1 | api/routes.py |
| api.rate_limit | 100 | api/limits.py |
| api.token | (none) | api/auth.py |
| api.token_ttl | 3600 | api/auth.py |
| cache.ttl | 300 | cache.py |
| db.pool_size | 10 | db/pool.py |
| db.timeout | 30 | db/pool.py |
| db.url | (none) | db/__init__.py, db/migrate.py |
| features | "" | util/feature.py |
| jobs.cron | */5 * * * * | jobs/schedule.py |
| jobs.queue | default | jobs/worker.py |
| jobs.workers | 4 | jobs/worker.py |
| log.format | json | log.py |
| mail.from | (none) | mail/send.py |
| mail.host | smtp.local | mail/send.py |
| mail.templates | templates/mail | mail/templates.py |
| metrics.port | 9100 | metrics.py |
| server.host | 0.0.0.0 | api/routes.py, app.py |
| server.port | 8080 | app.py |
EOF
