# Inventory counts, turnover and aging

Inventory counts are controlled operational documents. They reconcile the physical
quantity in one warehouse with the quantity and moving-average value in the inventory
ledger; they do not create general-ledger journals.

## Count lifecycle

A new count selects one or more existing warehouse/SKU balances and takes a frozen
snapshot of on-hand quantity, reserved quantity, quarantined quantity, inventory
value and average unit cost. The task then moves through these states:

1. `counting`: the scope is frozen and the operator records physical quantities.
2. `counted`: every selected line has a complete physical result and calculated
   variance, but the ledger has not changed.
3. `posted`: one atomic transaction records variance movements and updates inventory
   quantity, value and moving-average cost.
4. `cancelled`: the task is retained for audit and its scope is released without an
   inventory adjustment.

Creating, submitting, posting and cancelling a task records an immutable task event,
business audit record and transactional outbox event. Optimistic versions and
idempotency keys protect commands from stale or repeated submission.

## Frozen scope

An active `counting` or `counted` task freezes each selected warehouse/SKU balance at
the database boundary. Shipments, receipts, reservations, returns, scrap and opening
adjustments receive `INVENTORY_COUNT_SCOPE_FROZEN` until the count is posted or
cancelled. The posting transaction is the only permitted change within its own scope.

Posting locks the current balances and verifies that every snapshot still matches.
This makes a count variance reproducible even when several business services share
the inventory tables.

For shortages, inventory value is relieved at the frozen moving-average cost. For
surpluses, the frozen cost is reused when available; otherwise the operator must
provide a surplus unit cost. A zero-ending balance absorbs the exact remaining value
so quantity and value reconcile without a rounding residue.

## Operating indicators

The aging view measures days since the latest outbound inventory movement and groups
stock into `0–30`, `31–60`, `61–90` and `90+` day buckets. It is a movement-aging
indicator, not lot or expiry aging.

Monthly turnover is a management approximation: confirmed shipment cost divided by
current ending inventory value, with turnover days derived from the days in the
selected month. It is labelled as an operational indicator because it does not use a
general-ledger average-inventory balance.

## Boundary

The workflow owns physical count evidence, inventory variance movements, moving-
average inventory value and operating indicators. It deliberately excludes approval
accounting, journal entries, tax treatment, inventory provisions and statutory
financial statements.
