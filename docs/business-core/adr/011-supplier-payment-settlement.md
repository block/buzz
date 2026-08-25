# ADR 011: Supplier payment settlement

Status: accepted for B3.

Payment and payable projections are derived from append-only allocation and
reversal facts. This preserves attribution, reconciliation and concurrency
safety. Agent write tools remain forbidden: only a human BusinessSession may
invoke commands protected by CSRF, Origin, version, idempotency and scope.
