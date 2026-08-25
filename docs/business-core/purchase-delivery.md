# Purchase delivery and supplier performance

The purchase delivery control tower is an operational layer over confirmed
purchase orders. It does not create inventory, cost or payable facts. Those
facts remain authoritative only after a goods receipt is confirmed.

## Dates and commitments

`purchase_orders.expected_delivery_date` is the buyer's original planning
baseline. A supplier commitment is recorded separately in
`purchase_delivery_commitments` and has an ordered revision number. Replacing a
commitment supersedes the active revision; it never updates or deletes history.

Only confirmed purchase orders with an open receiving path accept a supplier
commitment. Commands require the active commitment revision, an idempotency key,
the `purchase_delivery_commitment:manage` permission and matching legal-entity,
supplier and business-unit scopes.

## Live delivery states

`purchase_delivery_current` derives its state at read time from the latest
supplier commitment (falling back to the original planned date), purchase-order
line quantities and confirmed goods receipts:

- `overdue`: open quantity remains after the promised date;
- `due_today` / `due_soon`: open quantity is due today or within three days;
- `on_track`: the open commitment is more than three days away;
- `completed_on_time` / `completed_late`: the final confirmed receipt date is
  compared with the promise;
- `unscheduled`: neither a supplier commitment nor an original planned date is
  available;
- `cancelled`: no further delivery is expected.

Draft or reversed receipts do not affect delivery completion. Partial receipts
reduce open quantity and remain visible as separate receipt batches.

## Supplier operating scorecard

The scorecard is calculated for a selectable 30/90/180/365-day order cohort:

- on-time completion rate = orders completed on or before promise / completed
  orders with a date baseline;
- quantity fulfillment rate = confirmed received quantity / ordered quantity;
- quality acceptance rate = 1 - confirmed purchase-return quantity / confirmed
  received quantity.

Rates with an empty denominator remain `null` and render as “—”. Quality is a
documented operational proxy based on actual purchase returns, not a subjective
supplier rating. No accounting entries or financial scoring are introduced.

## Browser workflow

The **交期履约** stage sits between purchase-order confirmation and goods
receipt. It shows delivery risk, a dated commitment track, partial-receipt
progress and supplier performance. Recording or revising a commitment uses a
modal form; goods receipt continues through the existing controlled receipt
workflow.
