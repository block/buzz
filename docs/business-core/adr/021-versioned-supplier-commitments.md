# ADR 021: Keep supplier commitments versioned and separate from purchase plans

## Decision

Keep the purchase order's expected delivery date as the original buyer plan.
Record supplier promises as immutable revisions, with exactly one active
revision per purchase order. Derive delivery risk and supplier performance from
those commitments plus confirmed receiving and return facts.

## Why

Overwriting the purchase order date would erase schedule drift and make a late
supplier appear on time after every renegotiation. A separate revision stream
preserves both the original plan and each supplier promise while allowing the
current control tower to use the latest commitment.

## Consequences

- Recommitment is explicit, optimistic and auditable.
- Delivery status changes naturally as the current date and confirmed receipts
  change; no scheduled status mutation is required.
- Supplier quality is reported from confirmed purchase returns. A future
  inspection module can add richer defect facts without changing this contract.
- The feature remains operational and does not create inventory, payable or
  ledger facts.
