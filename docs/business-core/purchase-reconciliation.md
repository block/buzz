# Purchase reconciliation

Inventory remains reconciled from append-only movements. B3 recomputes payable
settled/open and supplier-payment allocated/unapplied amounts from allocation
and reversal facts. Any non-zero difference blocks promotion and must be
corrected through business reversal commands.
