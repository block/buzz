# Concurrency

Expected versions reject stale document commands. Idempotency keys are scoped
by user and operation and bind to a canonical request hash. Inventory balances
are locked in stable warehouse/SKU order; receipt and receivable rows are locked
for allocation. Database checks are the final no-negative/no-overallocation
guard. The PostgreSQL test races two orders for common insufficient stock and
asserts exactly one confirmation succeeds.
