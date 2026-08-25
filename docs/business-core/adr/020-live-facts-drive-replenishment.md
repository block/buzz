# ADR 020: Derive replenishment from live inventory and supply facts

## Decision

Replenishment suggestions are a current projection, not stored recommendations.
Available inventory, confirmed purchase-order remainder and open purchase
requisitions are combined with a versioned safety-stock policy whenever the view is
read or a requisition is generated.

Generating a purchase requisition locks its selected policies, recomputes the
suggestion and stores the source quantities as immutable line snapshots. A confirmed
requisition closes only when it is linked to a purchase order that covers every SKU,
warehouse and requested quantity.

## Consequences

- Suggestions cannot remain stale after a reservation, shipment, receipt, return,
  purchase order or requisition changes supply.
- Inbound orders and open requisitions suppress duplicate demand.
- Operators retain a reproducible explanation for every requested quantity.
- Price negotiation and purchase-order confirmation stay in the existing purchasing
  workflow; replenishment does not introduce accounting behavior.
