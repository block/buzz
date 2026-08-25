# Session cleanup operations

The Gateway worker periodically marks expired binding challenges, Embed
sessions, and Business sessions. `SESSION_CLEANUP_INTERVAL_SECONDS` controls the
period. Failures emit a redacted warning and do not stop request serving.

For a manual sweep, run `business-auth-gateway --cleanup-once` with the normal
validated configuration, or use an audited DBA runbook. Never delete audit rows during cleanup.
Session records are first marked expired; physical deletion is a separate
retention job after the incident/audit window.

Readiness checks PostgreSQL; liveness checks only the process. Alert on cleanup
failure, replay/device rejection, database errors, and rate limits. Metrics and
logs must never include codes, cookies, bearer/session tokens, or full queries.
