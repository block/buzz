# Inventory reservations

One reservation belongs to a sales-order line, warehouse and SKU. Confirmation
requires the entire ordered quantity; B2 has no partial reservation/backorder.
Shipment consumes it, shipment reversal restores consumed quantity, and
cancel-remaining explicitly releases unused quantity. Balance and reservation
rows are locked for concurrent mutations.
