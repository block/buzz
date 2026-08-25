# Business data freshness

Every fact carries `sourceSystem`, stable object id/version, `updatedAt`,
`dataAsOf` and `sourceSyncStatus`. The acceptance snapshot is
`trade-acceptance-desensitized-v1`; it is not live production data.

`BUSINESS_DATA_STALE_AFTER_MINUTES` defaults to 1440. When snapshot age exceeds
that value the result is `partial`, contains `STALE_SOURCE_DATA`, and may not be
presented as current. Missing required profit components produce a
`profit_data_incomplete` Finding, `MISSING_COST_OR_EXPENSE` and low confidence;
the engine does not infer the missing values.

Production owners must define source-specific freshness SLAs, partial-sync and
status-conflict mappings, and measured outage behavior before READY. Freshness
must be evaluated from the authoritative source timestamp, not MCP receipt time.
