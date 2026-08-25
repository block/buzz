# Payable settlements

Allocation requires the same legal entity, supplier and currency. Payment and
payables are row-locked in stable order, then both projections and append-only
facts update atomically. Corrections insert reversal allocations; no original
fact is deleted or edited.
