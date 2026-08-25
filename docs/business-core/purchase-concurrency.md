# Purchase concurrency

PO lines are locked before receipt remainder is rechecked. Inventory balances
are locked by stable SKU order. Payables are locked by UUID order after the
supplier payment. These locks, numeric checks and optimistic versions prevent
over-receipt, lost value and negative open/unapplied amounts.
