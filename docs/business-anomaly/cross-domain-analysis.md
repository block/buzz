# Cross-domain analysis

Implemented joins are deliberately narrow:

- receivable to sales order by exact customer id for overdue-but-still-shipping;
- inventory to sales demand by exact SKU and warehouse for stockout risk;
- aged inventory to in-transit purchasing on the same inventory fact;
- order profit to overdue receivable by exact customer id for loss plus long term;
- purchase price history by exact SKU, supplier, currency and unit.

No name similarity, fuzzy matching or LLM inference is allowed. Missing stable
ids prevent the join. Scope checks are applied to every participating record,
so one authorized record cannot pull an unauthorized related record into a
Finding.
