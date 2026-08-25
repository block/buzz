# ADR 007: BusinessSession-only write boundary

Status: accepted for B2.

Native writes are human Business Web commands protected by BusinessSession,
CSRF, exact Origin, expected version, idempotency, rate limit and fresh B1
authorization. Agent/service credentials cannot call this boundary. Confirmed
documents are immutable and corrections use reverse commands so every change
remains attributable and auditable.
