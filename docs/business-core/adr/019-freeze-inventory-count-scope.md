# ADR 019: Freeze inventory count scope at the database boundary

## Decision

Creating an inventory count snapshots and freezes every selected warehouse/SKU
balance until the task is posted or cancelled. Database triggers reject both direct
balance mutations and inventory movements in an active count scope. The count-posting
transaction identifies itself with a transaction-local setting and is the sole
exception.

Posting re-locks every balance, verifies the frozen snapshot, records an immutable
`inventory_count_adjustment` movement and updates quantity, value and moving-average
cost in one transaction.

## Consequences

- Physical variances cannot be invalidated by concurrent reservation, shipment,
  receipt, return or opening operations.
- The invariant protects every current and future inventory writer, not only the
  inventory-count service.
- An active count temporarily blocks normal operations for its selected scope, so
  tasks should be narrow and promptly posted or cancelled.
- Variance value remains operational inventory authority and does not imply a
  general-ledger or statutory accounting posting.
