# One-Time Execution Grant Contract

V6.5 defines only `NonExecutableTestGrant`. Its status has one legal value:
`NON_EXECUTABLE_TEST_GRANT`. It stores a grant token hash, never a raw token,
and binds approval request, fixed action/capability version, resource and
Expected Version/state hash, approved payload hash, policy version, user,
approver decisions, audience, creation/expiry and trace.

A future V7 grant must use 256 bits of randomness, opaque presentation,
hash-only persistence, short expiry, audience binding, atomic one-time
consumption and revocation. The transaction that changes approval state,
creates an execution request and inserts its Outbox event must commit or roll
back as one unit.

No executable grant is signed or issued in V6.5. Any execution entry presented
with a test grant must return `BUSINESS_EXECUTION_NOT_ENABLED` and audit the
blocked attempt.
