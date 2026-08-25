# Purchase orders

Drafts may be replaced with an expected version. Confirmation freezes supplier,
lines, price, currency, terms and warehouse but posts no stock or payable.
`cancel-remaining` preserves received history and cancels only unreceived
quantity. Amounts use `NUMERIC(24,6)` and JSON decimal strings.
