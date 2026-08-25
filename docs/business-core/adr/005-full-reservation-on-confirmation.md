# ADR 005: Full reservation on confirmation

Status: accepted for B2.

Order confirmation requires every line fully reserved in one transaction.
Stable row locks and fail-closed shortages prevent overselling and ambiguous
partially confirmed orders. Backorders are deferred. Confirmed orders cannot be
edited; hold, shipment, cancel and reversal commands preserve history.
