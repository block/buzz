# B2 operations

1. Apply shared migration `0006_business_core_b2_sales_inventory_receivables.sql`.
2. Keep negative stock disabled and full reservation required; configure exact
   origins, cookie, numbering prefixes, currency and terms.
3. Load opening stock through idempotent create/post commands—never by writing
   balances directly.
4. Run both reconciliation endpoints and scoped read smoke tests.
5. Exercise confirmation, hold/release, partial shipment, receipt and allocation
   with a real BusinessSession before enabling users.
6. Monitor audit/outbox, conflict, shortage, forbidden, CSRF and rate-limit
   results. Correct mistakes using documented reversal ordering.
