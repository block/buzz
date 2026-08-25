# Profit facts

`profit_facts` is the append-only management-profit ledger. Shipment facts carry
revenue, frozen cost, quantity, all available dimensions, source event/version,
business period, watermark sequence, attribution and trace. Adjustment facts
point back to an immutable allocation. Reversal rows negate facts; facts are
never updated or deleted. Unique source-event/metric/line/direction keys make
projection replay idempotent.
