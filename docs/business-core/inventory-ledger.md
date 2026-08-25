# Inventory ledger

`inventory_movements` is the append-only authority for quantity and value.
Opening, shipment issue and explicit reversal types retain source IDs, cost
snapshots, actor and trace. `inventory_balances` stores on-hand, reserved,
moving-average cost and value as a transactionally maintained projection.
Database triggers reject updates and deletes of movement facts.
