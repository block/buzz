# Provisional purchase costing

Before supplier invoices exist, receipt inventory cost is the purchase-order net
amount excluding tax. Every receipt line stores `po_net_price`, `provisional`,
unit/total cost and snapshot time. Quantity and value are added before deriving
the new moving average. Historical shipment cost snapshots are immutable.
