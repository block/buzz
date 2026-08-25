# Operational returns

Sales and purchase returns are independent, append-only business documents. They are
not aliases for reversing an incorrect shipment or goods receipt.

## Sales return

A draft references one confirmed shipment and one or more of its lines. Draft and
confirmed returns reserve the source line's returnable quantity. Cancelling a draft
releases that quantity while retaining its event and audit trail.

Confirmation is atomic: inventory is received at the shipment's frozen unit cost,
the linked operational receivable is reduced by the original proportional sales
amount, and negative revenue/cost projection facts are queued. A return cannot be
confirmed for more than the receivable's open amount; settled value is never
silently rewritten.

Confirmed sales returns enter quarantine. Quarantined quantity remains on hand but
is excluded from sellable availability. Inspection disposes every line exactly once:
accepted quantity is released for sale, while scrap quantity and its frozen return
cost are removed from stock. The inspection record is immutable and retains the
operator, date, note and inventory movement trace.

## Purchase return

A draft references one confirmed goods receipt and one or more of its lines.
Confirmation removes available (unreserved) inventory at the current moving-average
cost and reduces the linked operational payable by the original proportional gross
amount. It fails if stock is unavailable or if the payable has already been settled
beyond the return amount.

After confirmation, the physical return can be marked dispatched with carrier and
tracking evidence, then supplier-acknowledged. These logistics transitions do not
change inventory or payable amounts.

## Operating metrics

The monthly return view reports sales and purchase return rates against same-month
confirmed shipment/receipt amounts. Sales return loss is a management measure:
returned sales amount minus returned product cost plus inspected scrap cost. It is
not a statutory loss or general-ledger balance.

## Boundary

These records govern inventory, operational receivables/payables and management
profit. They do not create journal entries, tax invoices, bank transactions or a
general-ledger posting model.

Confirmed returns are immutable in this release. Corrections use a new compensating
business document rather than editing confirmed quantities or amounts.
