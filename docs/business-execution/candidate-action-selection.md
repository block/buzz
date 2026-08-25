# Candidate Action Selection

## Decision

No V7 pilot action is selected. Selecting one would violate the hard gate
because no real Business System Capability API, Staging object, reversible
operation, idempotency contract, Expected Version contract or Postcondition
readback has been supplied.

| Referenced idea | Candidate status | Reason rejected now |
|---|---|---|
| `set_sales_order_manual_review_hold` | Not a discovered capability | Prompt example only; no upstream API evidence |
| Add internal review note/tag | Not a discovered capability | No versioned API or compensation evidence |
| Customer-wide shipment stop | Rejected | High risk and broader than one object |
| Credit limit change | Rejected | High financial/customer impact |
| Inventory, payment, invoice, journal or tax change | Rejected | High/Critical and explicitly outside the first pilot |

When real metadata arrives, exactly one fixed action may be selected only if
`BusinessWriteCapability::v7_pilot_eligible()` passes and manual review confirms
there is no money, invoice, accounting, tax, batch or cross-object effect.
