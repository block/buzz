# ADR 017: Model returns as independent operational documents

## Decision

Model customer and supplier returns as independent draft/confirmed/cancelled
documents linked to the original shipment or goods receipt. Do not reuse reversal
commands.

## Rationale

A reversal corrects an erroneous source posting and is valid only while downstream
facts permit complete erasure. A return is a later, valid commercial event after the
original fulfilment occurred. It therefore needs its own number, date, reason,
quantities, inventory movements and receivable/payable reduction.

Confirmation locks the return, its source lines, current inventory balance and the
operational balance in one transaction. Proportional commercial amounts use the
source document, while inventory uses the source shipment cost for a sales return
and current moving-average cost for a purchase return.

## Consequences

- Drafts reserve returnable source quantity and may be cancelled.
- Confirmed documents are immutable and auditable.
- Sales return projection facts reduce management revenue and product cost.
- No general-ledger, tax-invoice or bank-accounting behavior is introduced.
