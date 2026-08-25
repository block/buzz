# Business Write Capability Registry

The registry contract is `BusinessWriteCapability` in
`business-execution-contracts`. It is descriptive only and cannot carry an
endpoint, HTTP method or request body. Runtime discovery must use a fixed
Capability API and record one of `SUPPORTED`, `PARTIAL` or `UNSUPPORTED` for
each source-system capability.

Required fields cover risk, reversibility/compensation, Dry Run, Expected
Version, Idempotency, Postcondition readback, permissions, approver roles,
Step-up, environment support and API contract version.

## Current inventory

No authoritative capability endpoint was found, so the current registry has
zero discovered capabilities. The 21 V6 Action Catalog entries are human
review suggestions; they are not Business System write capabilities and must
not be copied into this registry as executable actions.

Generic actions such as `execute_any_action`, `generic_business_write`, dynamic
HTTP actions, SQL, Shell, RPA and browser automation are permanently rejected.
