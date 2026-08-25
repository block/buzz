# Sales orders

The server generates `SO-*` numbers. Only drafts can be replaced and replacement
requires the current version. Confirmation locks all balances in stable order
and creates every reservation atomically; any shortage rolls back the command.
Confirmed orders are immutable except for hold, release, shipment and
cancel-remaining commands. Cancel-remaining releases unconsumed reservations
while preserving shipped history.
