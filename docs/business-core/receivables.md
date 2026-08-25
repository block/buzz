# Operational receivables

B2 creates one receivable at shipment confirmation. It records legal entity,
customer, order, shipment, currency, original/settled/open amounts, due date and
version. Due date uses order terms, then customer terms, then
`DEFAULT_PAYMENT_TERMS_DAYS`. This is trade-operations truth, not an invoice,
subledger posting or general-ledger entry.
