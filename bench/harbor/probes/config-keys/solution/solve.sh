#!/bin/bash
cat > /app/KEYS.md <<'EOF'
| key | default | files |
|---|---|---|
| SVC_CACHE_URL | (none) | cache.py |
| SVC_LOG_LEVEL | info | log.py |
| SVC_METRICS | off | metrics.py |
| SVC_MIGRATE_DRY | (none) | db/migrate.py |
| SVC_SECRET_KEY | (none) | secrets.py |
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
| plugins.p01.limit | 232 | plugins/p01_hooks.py |
| plugins.p02.interval | True | plugins/p02_index.py |
| plugins.p03.limit | (none) | plugins/p03_index.py |
| plugins.p04.interval | on | plugins/p04_index.py |
| plugins.p05.target | 73 | plugins/p05_hooks.py |
| plugins.p06.enabled | eu-west | plugins/p06_sync.py |
| plugins.p07.interval | 320 | plugins/p07_hooks.py |
| plugins.p08.enabled | True | plugins/p08_sync.py |
| plugins.p09.interval | (none) | plugins/p09_sync.py |
| plugins.p10.enabled | eu-west | plugins/p10_export.py |
| plugins.p11.limit | 303 | plugins/p11_audit.py |
| plugins.p12.interval | /tmp/out | plugins/p12_export.py |
| plugins.p13.enabled | 340 | plugins/p13_hooks.py |
| plugins.p14.limit | False | plugins/p14_sync.py |
| plugins.p15.enabled | (none) | plugins/p15_hooks.py |
| plugins.p16.target | daily | plugins/p16_audit.py |
| plugins.p17.target | 16 | plugins/p17_index.py |
| plugins.p18.enabled | eu-west | plugins/p18_sync.py |
| plugins.p19.target | 198 | plugins/p19_sync.py |
| plugins.p20.enabled | True | plugins/p20_sync.py |
| plugins.p21.interval | (none) | plugins/p21_export.py |
| plugins.p22.limit | eu-west | plugins/p22_sync.py |
| plugins.p23.limit | 38 | plugins/p23_hooks.py |
| plugins.p24.interval | /tmp/out | plugins/p24_index.py |
| plugins.p25.enabled | 160 | plugins/p25_audit.py |
| plugins.p26.enabled | False | plugins/p26_audit.py |
| plugins.p27.interval | (none) | plugins/p27_sync.py |
| plugins.p28.enabled | on | plugins/p28_export.py |
| plugins.p29.limit | 409 | plugins/p29_sync.py |
| plugins.p30.interval | daily | plugins/p30_hooks.py |
| plugins.p31.interval | 375 | plugins/p31_hooks.py |
| plugins.p32.limit | False | plugins/p32_export.py |
| plugins.p33.limit | (none) | plugins/p33_sync.py |
| plugins.p34.interval | on | plugins/p34_hooks.py |
| plugins.p35.target | 454 | plugins/p35_audit.py |
| plugins.p36.interval | daily | plugins/p36_sync.py |
| plugins.p37.enabled | 22 | plugins/p37_hooks.py |
| plugins.p38.interval | False | plugins/p38_export.py |
| plugins.p39.enabled | (none) | plugins/p39_audit.py |
| plugins.p40.target | /tmp/out | plugins/p40_index.py |
| server.host | 0.0.0.0 | api/routes.py, app.py |
| server.port | 8080 | app.py, plugins/p07_hooks.py, plugins/p14_sync.py, plugins/p21_export.py, plugins/p28_export.py, plugins/p35_audit.py |
EOF
