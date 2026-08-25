# Data quality

Business facts expose source version, update/snapshot time and sync status.
Required profit inputs include product cost, freight, commission and customer
rebate in addition to revenue and the other available adjustments.

Missing required fields create `DATA-QUALITY-001` rather than a profit verdict.
The result is partial, confidence is low, and the anomaly audit emits
`BUSINESS_ANOMALY_DATA_QUALITY_BLOCKED`. Stale snapshots add
`STALE_SOURCE_DATA`. Unsupported profit-bridge structure effects remain zero
and the residual is returned as `unexplained_difference` with low confidence.

The acceptance fixture covers missing costs/expenses, partial sync and stale
handling. Real source mappings for missing currency, conflicting status and
broken foreign keys remain production integration gates.
