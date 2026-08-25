# Business Action Preflight Contract

`BusinessActionPreflight` is a read-only contract. It binds action, capability
version, resource ID/version/state hash, proposal hash, approval-draft hash,
permission version and approval-policy version. `executionAvailable` uses a
special type that rejects `true` during deserialization.

Allowed effects:

- read current object/version and permission/policy/capability metadata;
- invoke a statically allowlisted, documented side-effect-free `/preflight`;
- append a bounded Preflight audit record.

Forbidden effects include object or version changes, notes, holds, number
consumption, locks, notifications, workflows, ERP approvals or business
events. A real integration must compare a before/after snapshot and upstream
audit/event counters to prove this property.

No real Preflight was called in this workspace. Consequently there is no real
resource version, state hash or freshness claim, and the V7 gate remains
blocked.
