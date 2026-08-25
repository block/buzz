# Tool contracts

The canonical Rust schemas are in `business-query-contracts` and
`business-anomaly-contracts`. All input structs
use `deny_unknown_fields`; fields such as `sql`, `where`, `raw_filter`,
`raw_url`, `endpoint`, `path`, `script`, and `expression` cannot deserialize.

Limits:

- default/max page limit: 20/100;
- product ids: 50; warehouse ids: 20;
- customer/supplier/entity and other id lists: 50;
- default/max period: 90/366 days;
- bounded cursor and text length; control characters and traversal shapes fail.

Results use schema version 1 with status, `asOf`, effective scope summary,
summary, items, cursor pagination, server-generated resource references,
evidence, warnings, and Trace id. Exact-id authorization failures collapse to
`not_found_or_forbidden`.

V5 adds exactly eight tools: `search_business_anomalies`,
`get_business_anomaly`, `analyze_order_profit_risks`,
`analyze_receivable_risks`, `analyze_inventory_risks`,
`analyze_purchase_cost_risks`, `analyze_cross_domain_risks`, and
`explain_profit_change`. Their envelope includes a run id, Rule Set Version,
data snapshot, totals by currency, Finding count, severity/confidence,
threshold, Evidence and pagination. The model is never given generic join or
formula parameters.

Amounts are `{ "amount": "decimal-string", "currency": "CNY" }`; floats are
invalid. Quantities are decimal strings. Multi-currency amounts remain grouped
by currency unless the authoritative API supplies an explicit conversion.

Supported links are restricted to sales/purchase orders, inventory, customer,
supplier, and the two canonical subresources:
`biz://customer/<id>/receivables` and
`biz://supplier/<id>/payables`.
