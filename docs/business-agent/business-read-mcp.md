# Business Read MCP

`services/business-read-mcp` uses the repository's `rmcp` stdio stack. It has no
network listener and registers 28 read tools plus six fixed draft-creation
tools. The 14
authoritative Business Core reads are:

1. `get_sales_order`
2. `search_sales_orders`
3. `get_purchase_order`
4. `search_purchase_orders`
5. `query_inventory_balance`
6. `query_receivables`
7. `query_payables`
8. `query_order_profit`
9. `query_profitability_by_dimension`
10. `get_management_profit_report`
11. `get_management_report_snapshot`
12. `get_profit_evidence`
13. `get_operating_dashboard`
14. `get_business_data_quality`

The remaining read tools are eight deterministic anomaly reads and six
action-lifecycle reads. The write allowlist contains only sales order, shipment,
purchase order, goods receipt, customer receipt and supplier payment draft
creation. It cannot confirm, approve, execute, allocate, post, reverse or edit.

There is no SQL, arbitrary URL/path, generic API, Shell, filesystem, browser, or
generic write tool. Every call follows:

```text
strict input validation
-> atomically consume scoped Delegation
-> Business Read API with service credential + validated context
-> bounded JSON/result-schema validation
-> allowlisted biz:// link validation
-> success/partial/failure audit
```

Draft writes additionally receive a server-generated idempotency key derived
from the delegation and tool name, then pass through Business Core's existing
user permission, data-scope, validation, idempotency and domain-audit checks.

Default tool timeout is 10 seconds, result limit 100, payload limit 128 KiB.
The API client has no direct database access and does not forward Workbench,
Authentik, Embed, or Business Session credentials.

`BUSINESS_READ_ADAPTER=mock` is accepted only by debug builds and additionally
requires an explicit `Mock Only - Production Disabled` acknowledgement.
