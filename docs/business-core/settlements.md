# Settlements

Allocation locks receipt and receivable, verifies matching customer, legal
entity and currency, expected versions, unapplied amount and open amount, then
updates both projections in one transaction. Over-allocation fails closed.
Corrections append reversal allocations. Reconciliation recomputes receivable
and receipt totals from these immutable facts.
