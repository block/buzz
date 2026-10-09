# Community access verification evidence

Status: NOT RUN. Copy this template into a private evidence record. Replace
placeholders with verified facts; keep reusable credentials and payloads out.

## Deployment and ownership

- UTC start/end:
- Execution owner / setup-cleanup owner / deployment owner:
- Approved disposable community aliases and scope:
- Source SHA and prerequisite PR merge SHAs:
- Immutable image digest:
- Serving pod/runtime revisions and rollout state:
- Relevant membership / NIP-FI / accessory configuration:
- V1 cutoff (UTC or unset), last possible issuance, drain end, rollback versions:
- Candidate validation commands, exact HEAD, failures and skips:

## Case results

Use one row per principal/route/pod combination. Replace NOT RUN only after
capturing actual evidence. Include baseline and unban controls, not just denial.

| Case / fixture aliases | Runtime / revision | Expected | Actual | UTC / monotonic latency | Sanitized evidence | Result / follow-up owner |
|---|---|---|---|---|---|---|
| Baseline member/admin/owner-linked agent | | allowed | | | | NOT RUN |
| Outstanding v2 invite after issuer ban | | denied, no membership | | | | NOT RUN |
| Revoked v2 invite after issuer unban | | remains denied | | | | NOT RUN |
| New v2 invite after issuer unban | | allowed | | | | NOT RUN |
| Restricted claimant / owner-linked claimant | | denied | | | | NOT RUN |
| V1 current expiry/cutoff behavior | | deployed contract | | | | NOT RUN |
| Ban route matrix, one row per enforced route and principal | | denied / allowed controls | | | | NOT RUN |
| Timeout read/session/write controls | | write-only restriction | | | | NOT RUN |
| Root/audio direct and owner-linked existing sessions | | policy close / bystander stays | | | | NOT RUN |
| Root/audio reconnect | | denied | | | | NOT RUN |
| Tenant B controls | | unaffected | | | | NOT RUN |
| Cross-pod ordinary revocation | | recorded propagation | | | | NOT RUN |
| Missed-delivery local bound; staging if separately authorized | | documented bound | | | | NOT RUN |
| Database failure/grace isolated rehearsal | | bounded fail closed | | | | NOT RUN |
| Webhook ban / unban / wrong secret / timeout controls | | no denied run; allowed run | | | | NOT RUN |

## Cleanup and closure recommendation

- Fixture cleanup actions and owner-confirmed completion:
- Required failed/skipped cases and concrete follow-ups:
- Human verification confirmation (if any):
- Remaining hosted fleet, mixed-version, topology, and v1-drain requirements:
- Recommendation: PENDING (pass/fail/partial, evidence and scope):

A draft runbook, source tests, or merge is not deployed verification. Do not
claim complete fleet coverage from one staging run.
