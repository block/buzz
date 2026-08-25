# Customer receipts

The server generates `RCPT-*` numbers. Confirmation makes the full amount
unapplied. Allocations reduce it and produce partial/fully-allocated states. A
receipt can be reversed only after every allocation has an explicit reversal
fact. Receipt amounts, allocation totals and status are constrained together.
