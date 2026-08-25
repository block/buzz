# Business Execution V6.5

V6.5 is a production-readiness gate, not an execution release. The repository
contains no authoritative Business System Staging endpoint, production
permission service, enterprise directory, real test account, or reversible
write API evidence. The current decision is therefore **`V7_BLOCKED`**.

The only implementation added here is non-executable:

- strict capability, approval-policy, Preflight and test-grant contracts;
- a static read/preflight network allowlist;
- an environment guard that rejects `BUSINESS_EXECUTION_ENABLED=true`;
- a machine-computed V7 readiness gate;
- the checked-in honest evidence file used by that gate.

No execute, approve, reject, compensate, generic HTTP write, dynamic method or
dynamic URL route exists. V6 Acceptance adapters and pages are not promoted or
renamed.

Run:

```bash
. ./bin/activate-hermit
just business-v65-check
just business-v7-readiness  # expected exit 2 while V7_BLOCKED
```

Read [V7 readiness](v7-readiness.md), then the [candidate decision](candidate-action-selection.md)
and [threat model](threat-model.md).
