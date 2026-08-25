# Production Adapter

`business-read-mcp` accepts `BUSINESS_READ_ADAPTER=production`. Debug builds may
also use `mock` only with the exact acknowledgement string. Release builds
reject mock at startup; there is no fallback from production to fixtures.

## Request behavior

- fixed `POST /v1/read/{tool}` routes only;
- shared-secret reference authentication plus exact service audience;
- opaque Agent Delegation consumed only at the Gateway, never sent upstream;
- enterprise user, binding, delegation, Agent/Turn, used-call and Trace context
  sent as headers;
- connect/request timeout from bounded configuration;
- one retry for connection failures or 5xx only; 403/404 and 429 are never
  retried;
- 128 KiB default response cap and at most 100 rows/Findings;
- strict schema, decimal, Trace, Rule Version, Evidence and `biz://` validation.

The reference shared-secret mode is a deployment baseline, not the preferred
long-term workload identity. A production owner should replace it with mTLS or
short-lived signed workload credentials without exposing identity tokens to the
model.

## Failure mapping

403 and 404 become the same `not_found_or_forbidden` result. 429 becomes
`rate_limited`. Connection, timeout, 5xx, invalid JSON, schema mismatch,
oversized payload and invalid links become `upstream_unavailable`. Missing or
stale source fields remain structured `partial` rather than being invented.

The upstream API verifies delegation state before work and again before return.
If revocation wins the race, it drops the computed response.

## Deterministic rule set

`services/business-analytics/rules/trade-risk-v1.0.json` controls thresholds.
The API rejects a configured default version that does not equal the file
version. Implemented rule IDs are:

| Rule | Threshold / requirement |
|---|---|
| `PROFIT-LOSS-001` | contribution profit `< 0`; complete cost fields |
| `PROFIT-MARGIN-002` | margin `< 0.03`; equality does not trigger |
| `AR-SHIP-003` | overdue `>= 60d`, open `>= CNY 10,000`, continued shipping |
| `AR-UNINVOICED-004` | shipped and uninvoiced `>= 7d` |
| `AR-UNPAID-005` | invoiced and unpaid `>= 30d` |
| `PO-PRICE-006` | comparable median increase `>= 10%`, at least 3 samples |
| `PO-UNINVOICED-007` | receipt not invoiced `>= 30d` |
| `PO-PAYMENT-008` | payment/receipt progress gap `> 20%` |
| `INV-AGING-009` | age `>= 180d`, no sales in 90d; raises if still purchasing |
| `INV-STOCKOUT-010` | available below confirmed open-order demand |
| `INV-NEGATIVE-011` | on-hand or available `< 0` |
| `DATA-QUALITY-001` | required profit fields missing |
| `CROSS-LOSS-TERM-003` | loss order linked by stable customer id to overdue AR |
| `PROFIT-BRIDGE-001` | 11 named bridge effects; missing snapshots remain unexplained/low confidence |

Rules join only exact stable ids and comparable currency/unit values. They do
not use name similarity or model inference. The on-demand implementation is
stateless; scheduled runs and persisted finding lifecycle are intentionally not
implemented in this acceptance reference.
