# Goods receipts

A receipt belongs to one confirmed purchase order, legal entity, supplier and
warehouse. Confirmation locks PO lines and inventory balances, rejects
over-receipt, allocates net/tax/gross deterministically, posts purchase receipt
movements and creates one operational payable in the same transaction.
