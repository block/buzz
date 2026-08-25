# V7 Candidate Risk Matrix

There is no selected candidate. The rows below are risk hypotheses, not
capability claims.

| Idea | Business | Financial | Customer | Inventory | Fulfilment | Reversible | Concurrency | Permission/audit | Result |
|---|---|---|---|---|---|---|---|---|---|
| Sales-order manual-review hold | Medium | Low | Medium | Low | Medium | Unverified | Version race | Order scope, sales manager, Step-up, full audit | Blocked pending real API/compensation proof |
| Internal review tag/note | Low | Low | Low | None | Low | Unverified | Duplicate/idempotency | Object scope, reviewer, audit | Blocked pending real API proof |
| Customer-wide shipment stop | High | Medium | High | Medium | High | Operationally uncertain | Cross-order race | Broad scope, dual approval | Rejected for first pilot |
| Credit limit change | High | High | High | None | Medium | Context-dependent | Exposure race | Credit authority, dual approval | Rejected for first pilot |
| Inventory/payment/invoice/journal/tax | High/Critical | High/Critical | Varies | High where relevant | High | Often incomplete | Material | Specialist authority and strict audit | Rejected for first pilot |

No row may become `selected` until the source system proves a fixed inverse or
compensating action, actors, time window, expected final state and known cases
where compensation can fail.
