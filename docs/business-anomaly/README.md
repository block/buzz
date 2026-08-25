# Business Anomaly V5

This package describes the deterministic, read-only anomaly layer spanning
sales, purchasing, inventory, receivables, payables and order profit. Rules run
inside `services/business-analytics`; the language model only requests results,
explains returned Evidence and proposes non-executing review suggestions.

Current runtime evidence uses the versioned desensitized acceptance snapshot.
No authoritative ERP/WMS/finance API is connected, so the implementation is a
production-shaped acceptance reference and is not READY for customer data.

See [architecture.md](architecture.md), [rule-reference.md](rule-reference.md),
[data-quality.md](data-quality.md) and [operations.md](operations.md).
