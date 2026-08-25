# ADR 001: V7 Readiness Gate

## Status

Accepted for V6.5; current result `V7_BLOCKED`.

## Decision

V7 readiness is computed from 23 explicit hard conditions and evidence
references. The input cannot declare its own decision. Acceptance/Mock
environments cannot pass even if every boolean is changed to true.

## Consequences

The project stops before write development until real-system, permission,
directory, page, candidate, policy, identity, recovery, audit and Kill Switch
evidence exists. This trades schedule certainty for an honest authority
boundary and prevents a test-complete pseudo-production loop.
