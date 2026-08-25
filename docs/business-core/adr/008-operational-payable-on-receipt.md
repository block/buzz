# ADR 008: Operational payable on receipt

Status: accepted for B3.

Fulfillment of a purchase commitment occurs at confirmed receipt, so inventory
and an operational payable are created atomically then. This prevents stock
without a commercial obligation. The payable is explicitly not a supplier
invoice, tax record, subledger or general-ledger posting.
