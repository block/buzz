# V7 Readiness

## Current result

`V7_BLOCKED`, evaluated on 2026-08-20. The decision is computed from
[`v7-readiness-evidence.json`](v7-readiness-evidence.json); no decision field is
accepted as input. A true condition also needs at least one evidence reference,
and the candidate list must contain exactly one fixed action code.

| Hard condition | Result | Current evidence or blocker |
|---|---:|---|
| real_business_system_connected | FAIL | No authoritative Staging/Sandbox system or endpoint |
| production_read_adapter_ready | FAIL | Existing adapter targets a desensitized reference API |
| production_permission_service_ready | FAIL | Acceptance allowlists only |
| enterprise_directory_ready | FAIL | No directory endpoint or credentials |
| assignee_resolver_ready | FAIL | Current-user-only Acceptance resolver |
| approver_resolver_ready | FAIL | Not connected |
| real_business_dock_pages_ready | FAIL | Mock/Acceptance pages only |
| candidate_action_selected | FAIL | Selection forbidden without real capability evidence |
| candidate_action_low_or_medium_risk | FAIL | No selected action |
| candidate_action_reversible | FAIL | No compensation proof |
| candidate_action_single_object | FAIL | No selected action |
| candidate_action_staging_supported | FAIL | No Staging API |
| candidate_action_idempotency_supported | FAIL | No upstream contract evidence |
| candidate_action_expected_version_supported | FAIL | No upstream contract evidence |
| candidate_action_postcondition_supported | FAIL | No upstream contract evidence |
| approval_policy_ready | FAIL | Contract only; no authoritative policy |
| separation_of_duties_ready | FAIL | No real approver resolution |
| step_up_auth_ready | FAIL | No Authentik step-up POC evidence |
| service_identity_ready | FAIL | Shared-secret baseline; no Staging workload identity |
| staging_reset_or_recovery_ready | FAIL | No real Staging recovery drill |
| audit_ready | FAIL | Event names defined; no V7 runtime audit integration |
| kill_switch_ready | FAIL | Configuration contract only; no execution service to enforce it |
| business_execution_disabled | PASS | V6 has no execute API; V6.5 config guard rejects `true` |

## Required external inputs

| Blocker | Responsible system/team | Needed input | Release condition |
|---|---|---|---|
| Business Staging | ERP/WMS/Finance owners | Origin, API versions, isolated test objects, accounts, reset SLA | Six read domains pass contract tests |
| Permission service | Business IAM | Endpoint, workload audience, permission version and at least three scope dimensions | Two users show different scopes and 404 denial |
| Directory | Enterprise identity | Stable EnterpriseUser mapping, active/role/org/scope queries | Assignee and approver resolution pass fail-closed tests |
| Write capabilities | Business API owners | Versioned read-only capability metadata and side-effect-free Preflight | Exactly one eligible reversible fixed action is proven |
| Step-up | Identity/Auth team | Authentik authentication context, MFA and validity window | Approval candidate can be verified without treating login as step-up |
| Recovery | Staging operations | Snapshot/reset/rebuild runbook and access | Timed recovery drill is recorded and repeatable |
| Service identity | Platform security | mTLS, workload identity or short-lived service JWT | Rotation and revocation tested with a distinct audience |

Changing JSON booleans without producing the referenced runtime evidence does
not make the system safe and is not an approved release procedure.
