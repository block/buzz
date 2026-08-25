# ADR 004: Inventory movement authority

Status: accepted for B2.

Inventory movements are authoritative quantity/value facts; balances are a
lockable projection. This makes history, cost snapshots and reconciliation
reproducible. Confirmed facts cannot be edited. Every correction is an explicit
opposite movement linked to its source because mutable balances alone cannot
explain who changed stock or why.
