# Feature 13 — ORBIT Auth Server, Device Identity & Data Governance

> **Priority:** P1 for the hosted product; required before public cloud sync or enterprise rollout.
> **Scope:** identity/control plane, device registration, user tracking boundaries, enterprise workspace governance, policy enforcement.

## 1. Auth server boundary

ORBIT should maintain a dedicated hosted authentication/control-plane service. The service can be deployed separately from the memory data plane so account credentials do not share infrastructure or schemas with brain content.

### Responsibilities

- signup with email address + password
- password hashing and credential lifecycle
- email verification and account recovery
- login/logout/session management
- subscription/entitlement state
- organization/workspace membership
- desktop authorization-code issuance
- device registration/revocation
- security audit events
- optional product telemetry keyed by non-sensitive IDs

### Desktop boundary

The Tauri application never needs the user's plaintext password. It opens the hosted web UI in the system browser for signup/login, receives a short-lived deep-link authorization result, and stores the resulting session material in the operating-system credential store.

## 2. First-launch UX

```text
ORBIT start
  │
  ├── Continue Local ─→ create/use local brain
  │
  ├── Log In ─────────→ browser → auth → deep link → desktop
  │
  └── Sign Up ────────→ browser → email/password/verification
                                      │
                                      ↓
                               account created
                                      ↓
                              subscription choice
                                      ↓
                               device registered
                                      ↓
                                desktop returns
```

If a valid local session is present, ORBIT may skip the sign-in screen and open the last authorized workspace, subject to local security settings. Logout revokes the cloud session but does not automatically erase local memory.

## 3. Account record versus brain record

### Account/control-plane record

Contains identity and product-access information, for example:

- account ID
- verified email
- password credential hash / authentication provider reference
- profile metadata
- subscription/entitlement state
- organization memberships
- consent/privacy settings

### Device record

Contains:

- random `device_id`
- public key
- platform and OS version
- ORBIT version
- device label
- last-seen / sync metadata
- optional purpose-limited derived device fingerprint

Do not store raw MAC addresses by default. A raw MAC is not an appropriate sole authentication factor and should not become a permanent cross-service identifier.

### Brain/workspace record

Contains the ORBIT memory system: documents, chunks, memories, decisions, graph relationships, provenance, sessions and derived indexes. These are governed by workspace policy and must remain logically separate from identity telemetry.

## 4. Tracking / telemetry policy

Product analytics may track account/device lifecycle events, but it must not become an indirect channel for source-code or memory exfiltration.

Allowed by default:

- app version
- platform family
- crash/health diagnostics
- login/device registration result
- sync latency/failure category
- feature counts without content

Disallowed by default:

- source code
- prompts/responses
- embeddings
- raw file paths
- memory text
- MCP payloads
- raw MAC address

Enterprise administrators should be able to disable optional product analytics for managed devices/workspaces.

## 5. Workspace/ownership model

Every cloud-capable logical record must resolve to:

```text
owner_type: user | organization
owner_id
tenant_id
workspace_id
project_id (optional)
privacy_class
sync_mode
processing_mode
retention_policy_id
residency_region
policy_version
```

Personal and enterprise spaces are not simply folders. They are different authorization domains.

### Personal workspace

Owned by the user. Organization membership does not grant access. Cloud sync is optional and user-controlled.

### Enterprise workspace

Owned/managed by the organization. Access is controlled by organization and project ACLs. Enterprise policies may constrain sync, retention, processors, residency and export.

## 6. Policy enforcement pipeline

The policy engine must run before each sensitive transition:

```text
INGEST
 ↓
Classify + secret scan
 ↓
Resolve owner / tenant / workspace
 ↓
Evaluate WorkspacePolicy
 ↓
Local processing
 ↓
Store local state
 ↓
[cloud sync gate]
 ↓
[hosted processing gate]
 ↓
Permission-aware retrieval
 ↓
Context pack / agent export
```

The same policy object should be available to ingestion workers, embedding workers, graph builders, sync workers and retrieval. This prevents a connector from bypassing governance by using a different code path.

## 7. Policy fields

Minimum configurable controls:

- data classification
- owner/tenant/workspace/project
- local-only paths/sources
- cloud-sync allow/deny
- hosted-processing allow/deny
- allowed model providers
- allowed connectors
- data residency
- retention duration / preservation hook
- deletion/export behavior
- secret detection action
- audit level
- policy version

## 8. Deletion and lifecycle

Support deletion at all layers:

```text
account / workspace policy
        ↓
logical delete event
        ↓
local memory tombstone
        ↓
cloud tombstone
        ↓
object-store deletion
        ↓
index/graph cleanup
        ↓
backup retention workflow
```

Deletion must be idempotent and must not allow an older device or delayed event to resurrect a deleted enterprise record.

## 9. Enterprise processing states

```text
LOCAL_ONLY
  Storage: local
  Compute: local
  Cloud: none

CLOUD_SYNC
  Storage: local + encrypted cloud replica
  Compute: local

ENTERPRISE_MANAGED
  Storage: local + organization-controlled cloud replica
  Compute: local by default
  Policy: organization controlled

HOSTED_PROCESSING
  Storage: cloud replica
  Compute: selected ORBIT services may process remotely
  Policy: explicit organization/user authorization
```

These states should be visible in desktop settings and diagnostics.

## 10. Acceptance tests

- [ ] Fresh install can remain local without creating an account
- [ ] Sign Up always opens the ORBIT auth website
- [ ] Login/password handling never occurs in the desktop UI process
- [ ] Existing session can reopen securely
- [ ] Deep-link callback cannot inject a session without valid state/PKCE/code exchange
- [ ] Device revocation prevents further cloud sync
- [ ] Personal workspace is invisible to enterprise admins unless explicitly shared
- [ ] Enterprise workspace is inaccessible outside its ACLs
- [ ] A `LOCAL_ONLY` memory never enters a cloud sync batch
- [ ] A `HOSTED_PROCESSING` request is rejected when workspace policy disallows it
- [ ] Audit records identify policy decisions without storing sensitive memory content
- [ ] Deletion tombstones propagate across devices
