# V7 Write-Path Threat Model

| Threat | Required mitigation | Current status |
|---|---|---|
| Agent requests/attempts a write | No write tools/scopes; execution route absent | PASS for V6.5 |
| Forged action code or resource ID | Fixed capability/action allowlists; authoritative 404 scope check | Contract only |
| Forged approval/self-approval | Server policy snapshot, directory resolution, separation of duties | Blocked |
| Object changes after approval | Bind Expected Version and state hash; re-read before use | Contract only |
| Duplicate/concurrent/timeout retry | Upstream idempotency, atomic grant consumption, readback reconciliation | Blocked |
| Service identity leakage | Short-lived workload identity, exact audience, log redaction | Blocked |
| Idempotency-key collision/replay | Bind key to approved payload/action/resource; reject mismatched reuse | Contract only |
| Execution Grant replay | Opaque 256-bit token, hash-only storage, atomic single consume, short expiry | Design only |
| User/binding/session/permission revocation | Re-evaluate immediately before any future grant/operation | Blocked |
| Step-up expiry | Bind context and issued/expiry times to approval | Blocked |
| Upstream succeeds after local timeout | Query transaction/idempotency status and Postcondition before retry | Blocked |
| Postcondition mismatch | Stop, alert and preserve before/after evidence | Contract only |
| Compensation failure | Explicit owner/runbook/escalation; never promise automatic rollback | Blocked |
| Audit tampering | Append-only constrained audit plus external retention/verification | Partial V6 only |
| Kill Switch failure | Default-on, independent enforcement, negative runtime test | Config contract only |
| Dynamic URL/method abuse | Exact origins; GET plus four side-effect-free POST paths only | PASS in contract tests |

Further concerns include confused-deputy use of BusinessSession, permission
cache crossing users, stale permission versions, schema smuggling, prompt
injection in business text, side effects hidden behind nominal Preflight, and
Production/Acceptance environment confusion. All fail closed; no Mock fallback
is permitted.
