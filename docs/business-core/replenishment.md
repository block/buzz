# Inventory replenishment and purchase requisitions

Replenishment is an operational planning workflow built on authoritative inventory
and purchasing facts. It does not create accounting entries or supplier invoices.

## Safety-stock policy

One active policy governs one legal-entity, warehouse and SKU combination. It stores
the preferred supplier, base unit of measure, safety stock, reorder point, target
stock, minimum order quantity, order multiple and expected lead time.

Thresholds are deliberately ordered:

`0 ≤ safety stock ≤ reorder point < target stock`

Policy replacement requires the current optimistic version. Every change is written
to the business audit log and transactional outbox.

## Replenishment calculation

The current view derives these quantities from live business facts:

- Available = on hand − reserved − quarantined.
- Inbound = confirmed purchase-order quantity not received or cancelled.
- Open requisition = draft or confirmed purchase-requisition quantity.
- Projected = available + inbound + open requisition.

When projected quantity is at or below the reorder point, the suggested quantity is
the larger of the minimum order quantity and the target-stock gap rounded upward to
the configured order multiple. Open purchase orders and requisitions therefore
prevent duplicate suggestions.

Risk labels distinguish an uncovered safety-stock breach, an uncovered reorder
warning, coverage by inbound orders, coverage by a purchase requisition, a healthy
balance and a paused policy.

## Purchase-requisition lifecycle

A generated requisition contains only policies for one legal entity, warehouse and
preferred supplier. Creation locks the policies, recomputes every suggestion and
freezes the current available, inbound, open-requisition, reorder-point and target
quantities on each line.

The lifecycle is:

1. `draft`: generated from current actionable suggestions.
2. `confirmed`: accepted into the operating purchase plan.
3. `converted`: linked to a draft or confirmed purchase order that covers every SKU,
   warehouse and requested quantity.
4. `cancelled`: releases its planning coverage so suggestions recalculate.

Draft or confirmed requisitions can be cancelled. Only confirmed requisitions can be
converted. Conversion validates purchase-order coverage at the database transaction
boundary and retains the immutable requisition and event trail.

## Boundary

The workflow governs stock policy, shortage risk, replenishment recommendations and
purchase demand. Purchase-order prices remain owned by the purchasing workflow. No
general-ledger, accrual, provision, invoice or statutory financial behavior is added.
