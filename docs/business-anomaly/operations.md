# Operations

Deploy the Gateway migration through `0003`, the Business Read API, analytics
rules and `business-read-mcp`. Use `BUSINESS_READ_ADAPTER=production`; release
builds reject mock. Configure the service audience, authoritative API URL,
authorization enablement, rule path/version, stale threshold, 128 KiB cap,
finding cap and bounded tool/run timeouts as documented in
`../business-agent/operations.md`.

Scheduling is not implemented and `BUSINESS_ANOMALY_SCHEDULE_ENABLED=true`
fails startup. Monitor authorization denials, partial/data-quality outcomes,
upstream failures, rule version, duration and response-audit continuity. Audit
stores aggregate counts and ids, never answer text or raw business detail.

Production cutover requires real endpoint/workload identity, real permission
resolution, fresh source SLA, live Agent-to-Buzz response evidence, macOS and
Windows ResourceRef navigation, and representative load results.
