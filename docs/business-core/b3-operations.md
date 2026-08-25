# B3 operations

1. Apply migration `0007_business_core_b3_purchasing_payables.sql`.
2. Keep over-receipt disabled and cost policy fixed to provisional PO net.
3. Run `just business-b3-check` and a disposable PostgreSQL B3 E2E.
4. Verify inventory and payable reconciliation are consistent.
5. Exercise the real BusinessSession/CSRF flow and Business Dock embeds.
6. Correct mistakes through allocation/payment/receipt reversal ordering.
