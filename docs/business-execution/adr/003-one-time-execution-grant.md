# ADR 003: One-Time Execution Grant

## Status

Contract design only; not executable in V6.5.

## Decision

Future execution authorization will be an opaque, 256-bit, short-lived,
hash-only, audience-bound, action/resource/version/payload/policy/decision-bound
token consumed atomically once. Approval state, execution request and Outbox
insert share one transaction.

## Consequences

Session presence, Agent Delegation, Workbench login, an approval-draft ID or a
replayed decision can never substitute for a grant. V6.5 test grants have the
single `NON_EXECUTABLE_TEST_GRANT` status and every execution attempt remains
blocked.
