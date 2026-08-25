# Business Dock integration

The resolver allowlists `biz://anomaly/<id>`, `biz://action-proposal/<id>`, `biz://work-item/<id>`, and `biz://approval-draft/<id>` and maps them to `/embed/anomalies/`, `/embed/action-proposals/`, `/embed/work-items/`, and `/embed/approval-drafts/`. IDs, origin, traversal, fragments, and query strings are strictly validated.

The acceptance fixture is labeled `Desensitized Acceptance UI` and `Production Disabled`. It shows finding detail, proposal detail, work-item preview, confirmed work item, and approval-draft detail. The approval page carries a fixed Draft Only warning.

Bridge V2 accepts only `work_item_created`, `work_item_status_changed`, `approval_draft_created`, `approval_draft_updated`, and `finding_acknowledged` action notifications. The Buzz host may toast, update the current resource, and refresh read queries. It does not publish to another channel, advance an Agent, approve, or call a business write.

## Conversation-started data entry

Business Agent replies may include one of six data-free entry references:
`biz://sales-order-entry`, `biz://shipment-entry`,
`biz://purchase-order-entry`, `biz://goods-receipt-entry`,
`biz://customer-receipt-entry`, or `biz://supplier-payment-entry`.
Clicking one opens the matching human Business Web form in Business Dock.
The reference contains no form values, token, query, or fragment; conversation
text is not treated as authoritative input and is never submitted
automatically. The user must verify and submit the form through the existing
BusinessSession-only write boundary, so CSRF, authorization, idempotency,
rate-limit, versioning, and audit controls remain in force.
