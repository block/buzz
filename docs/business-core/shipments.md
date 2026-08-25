# Shipments

A draft references one confirmed, non-held sales order and one valid warehouse.
Lines cannot exceed unshipped reserved quantities. Confirmation atomically
consumes reservation, decreases stock/value, stores average-cost snapshots,
advances order shipped quantities and creates an operational receivable.
Reversal uses opposite movements and is blocked after settlement; allocations
must be reversed first.
