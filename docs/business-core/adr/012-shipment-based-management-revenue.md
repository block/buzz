# ADR 012: Shipment-based management revenue

Accepted. Management revenue begins when shipment is confirmed because that is
the existing operational fulfillment and receivable boundary. Order confirmation
is only a commitment. Historical product cost uses the shipment cost snapshot,
not today's moving average, so later receipts cannot rewrite earned margin.
