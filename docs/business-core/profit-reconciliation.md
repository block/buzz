# Profit reconciliation

`profit_projection_reconciliation` compares each shipment line's authoritative
sales amount and frozen cost snapshot with signed projected facts. The endpoint
returns only revenue/cost differences, fact counts and last watermarks. An empty
difference set is `consistent=true`; it is not a general-ledger reconciliation.
