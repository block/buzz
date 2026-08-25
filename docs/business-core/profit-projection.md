# Profit projection

The worker consumes `shipment_confirmed` and `shipment_reversed` outbox events in
created-at/id order. Confirm produces normal net-revenue and product-cost facts;
reverse produces matching reversal facts. Attribution is inherited from the
shipment domain event. Offsets, retryable failure rows and idempotent fact keys
support restart and replay. `rebuild` resets only the consumer offset—it does not
delete immutable facts.
