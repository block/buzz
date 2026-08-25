# Reconciliation

`/v1/reconciliation/inventory` compares balance projections with summed
movement quantity/value and active reservations.
`/v1/reconciliation/receivables` compares receivable/receipt projections with
append-only allocations. Non-empty results block promotion. Operators correct
source facts through reversal commands, never direct SQL edits.
