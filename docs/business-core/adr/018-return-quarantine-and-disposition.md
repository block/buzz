# ADR 018: Quarantine returned stock until inspection

## Decision

Confirmed customer returns increase on-hand inventory and quarantined quantity in the
same transaction. Available inventory is `on hand - reserved - quarantined`.
Inspection must classify every returned line as accepted or scrap exactly once.

Accepted stock releases quarantine without changing quantity or value. Scrap reduces
on-hand quantity and inventory value at the return's frozen shipment cost and records
an immutable inventory movement.

Supplier returns retain their confirmation-time inventory/payable effects and add
separate dispatched and supplier-acknowledged logistics transitions.

## Consequences

- Uninspected returns cannot accidentally satisfy a sales reservation.
- Inventory value remains traceable through quarantine and scrap.
- Operational return rates and loss metrics can be reported without adding general
  ledger, invoice or bank-accounting behavior.
