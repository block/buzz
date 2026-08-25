# ADR 013: Profit facts as a rebuildable projection

Accepted. Shipment events remain the authority; profit facts are an append-only,
idempotent analytical projection with source event/version and offsets. The
projection can replay without changing historical facts, while reconciliation
detects omission or drift. This separates operational transactions from evolving
management-profit read models.
