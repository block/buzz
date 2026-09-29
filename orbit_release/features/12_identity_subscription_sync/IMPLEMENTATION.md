# Feature 12 — Identity, Subscription & Multi-Device Sync

> **Priority**: P1 — hosted product foundation after local V1.
> **Dependencies**: Feature 10 and Feature 11.

## Goal

Provide a normal desktop-app account experience while preserving ORBIT's local-first privacy model. The desktop application may show **Log In** and **Sign Up** on first launch, but the actual account creation, password handling, email verification, subscription management and identity proofing happen on the ORBIT web authentication server.

The desktop app stores a secure local session after login, so returning users can reopen ORBIT like a normal native application. The account session is not the memory database and must not be mixed with brain/workspace data.

### Product entry flow

```text
Open ORBIT
    ↓
┌─────────────────────────────────────────┐
│ Log In  |  Sign Up  |  Continue Local  │
└─────────────────────────────────────────┘
    │             │             │
    │             │             └────────────→ Local brain only
    │             │
    │             └──────────────────────────→ ORBIT Auth Website
    │                                          email + password
    │                                          verification / recovery
    │                                          subscription / account
    │                                                │
    └───────────────────────────────────────────────┘
                     Browser callback / deep link
                              ↓
                     Desktop code exchange
                              ↓
                     Local secure session
                              ↓
                  Device registration / policy
                              ↓
                  Optional Cloud Sync entitlement
```

**Important:** an account is an identity/control-plane object. A user's local brain remains local unless the user explicitly enables synchronization or an enterprise workspace policy authorizes another storage/processing path.

## 0. Authentication server responsibilities

ORBIT should operate a dedicated **web authentication/control-plane service** rather than implementing account registration inside the Tauri application. The service is responsible for:

- signup with email address and password
- email verification and account recovery
- login/logout/session lifecycle
- subscription and entitlement state
- account profile metadata
- device registration and revocation
- authorization for personal and enterprise workspaces
- issuing short-lived authorization codes for desktop deep-link callbacks
- security/audit events for identity operations
- optional privacy-preserving product telemetry associated with a user/device ID

The authentication service should expose the smallest identity payload required by the desktop client. It should not receive raw local memory, embeddings, IDE transcripts or source-code content merely because the user has an account.

### Desktop login state

After a successful login, the desktop stores only the minimum session material required to reopen the account securely:

- account/user ID
- session/refresh credential reference stored in the OS keychain
- selected workspace ID(s)
- entitlement cache
- local `device_id`
- device public key / registration metadata

Never store the user's plaintext password in the desktop app. Never place long-lived tokens in a deep-link URL.

### Device metadata and hardware fingerprinting

ORBIT may collect a small amount of device metadata for security, abuse prevention, device management and future product analytics, but this data must remain separate from memory/workspace content.

Recommended device record:

| Field | Purpose | Handling |
|---|---|---|
| `device_id` | Stable logical installation identity | Random, non-secret ID |
| `device_public_key` | Device proof / signed requests | Public; private key in OS keychain |
| `platform` / `os_version` | Compatibility and security telemetry | Minimal retention |
| `app_version` | Support / upgrade tracking | Minimal retention |
| `device_label` | User-visible device management | User editable |
| `hardware_fingerprint` | Optional anti-abuse / risk signal | Prefer salted/derived value; never rely on it as the sole authenticator |
| raw MAC address | Avoid as a primary identifier | Do not store by default |

A MAC address is a network-interface identifier, not a secure identity secret. It can be unavailable, change across interfaces/environments, or be restricted by operating-system privacy controls. If hardware characteristics are used, derive a purpose-limited device risk/fingerprint identifier and document its retention and user-facing purpose.

### User tracking / telemetry

Future product analytics should use `account_id` + `device_id` rather than memory contents or raw hardware identifiers. Telemetry should be category-based and configurable, for example:

- app version and crash diagnostics
- login/device registration events
- sync success/failure and latency
- feature usage counts
- aggregate retrieval performance

Do not send source-code text, prompts, embeddings, memory contents, file paths or MCP payloads as analytics unless the user or enterprise administrator has explicitly enabled a feature that requires that transfer.

## 1. Website-first authentication

Do not build password or signup forms inside the desktop app.

Use the desktop app as an OAuth public client:

1. Generate a high-entropy state and PKCE verifier.
2. Open the authorization URL in the default external browser.
3. User authenticates on the website.
4. Website redirects to an ORBIT-owned HTTPS app/universal link where supported, or a reverse-domain private-use scheme fallback.
5. Desktop validates state and PKCE binding.
6. Desktop exchanges the short-lived authorization code for a native session.
7. Refresh/session credentials go into the operating-system keyring.

RFC 8252 recommends the external browser flow for native apps and requires PKCE for public native clients. citeturn658627search0turn658627search1

Tauri's deep-link support can register both custom desktop schemes and HTTPS app/universal links. citeturn658627search4

## 2. Device registration

Each installation receives a random `device_id` and a device key pair.

Register:

- `device_id`
- account/user ID
- workspace memberships
- platform and app version
- device public key
- last-seen timestamp
- sync cursor
- capability profile

Never use a device ID as a secret.

Allow the user/admin to revoke individual devices. Revocation must prevent future cloud sync while leaving the local brain intact until the user deliberately removes it.

## 3. Subscription entitlement

Subscription state belongs to the account/workspace, not the local database.

Suggested entitlement capabilities:

| Capability | Local | Cloud Sync | Enterprise |
|---|---:|---:|---:|
| Local memory | Yes | Yes | Yes |
| Local retrieval | Yes | Yes | Yes |
| Account/device registration | No | Yes | Yes |
| Cloud memory sync | No | Yes | Yes |
| Multi-device continuity | No | Yes | Yes |
| Organization policies | No | Limited | Yes |
| Hosted processing | Optional external provider | Optional | Policy controlled |

The entitlement cache should be short-lived and signed/server-verifiable. Loss of network access must not immediately erase or disable local memory.

## 3A. Personal versus enterprise workspaces

A signed-in user can have multiple logical workspaces. At minimum ORBIT should support:

1. **Personal workspace** — the user's private brain and personal memory.
2. **Enterprise workspace** — organization-owned data, policies and memories.
3. **Project/workspace scopes** inside an enterprise tenant — repository, team, customer, department or other bounded context.

The ownership boundary must be a first-class field in every canonical record:

```text
owner_type: user | organization
owner_id:   account_id | organization_id
tenant_id:  ...
workspace_id: ...
project_id: ...
privacy_class: local_only | personal | enterprise | public
processing_policy: local_only | cloud_sync | hosted_processing
retention_policy_id: ...
```

A user being a member of an enterprise organization must **not** make their personal workspace enterprise-visible. Conversely, enterprise memory must not be silently copied into a user's personal brain. Retrieval must enforce the workspace and ownership boundary before candidate generation or graph expansion.

## 4. Canonical sync model

The cloud replica synchronizes **logical records and events**, not local database files.

### Canonical records

- memory items
- decisions
- preferences
- provenance
- source references
- session summaries selected for sync
- workspace metadata
- deletion/forget events

### Derived records

- embeddings
- FTS indexes
- graph materializations
- caches
- reranker features

Derived records can be regenerated on every device. They may optionally be transferred as optimization artifacts when the model/index version matches.

## 5. Event format

Each logical event contains:

```text
EventID
WorkspaceID
DeviceID
EntityID
EventType
SchemaVersion
LogicalTime
ObservedAt
ContentHash
ParentEvents / CausalVersion
Payload
EncryptionMetadata
```

Use idempotent event IDs and a deterministic apply operation. A duplicated event must produce no additional state change.

## 6. Conflict handling

Use different policies by memory type:

- Append-only session events → merge by event ID.
- User preferences → explicit last-write-wins only within the same preference scope, with version metadata.
- Architectural decisions → never silently overwrite; create a new decision and mark the older one superseded.
- Source documents → conflict copy / revision record, never destructive overwrite.
- Deletions/forget events → tombstones with causal/version metadata so a deleted item cannot resurrect during an older-device sync.

Do not solve every conflict with a generic database `UPDATE`.

## 7. Multi-device flow

```text
Device A                         Cloud                         Device B
   │                              │                              │
   │ local memory event           │                              │
   ├──────── encrypted event ────►│                              │
   │                              │ validate/store                │
   │                              │◄──── sync cursor request ─────┤
   │                              ├──── encrypted events ───────►│
   │                              │                              │
   │                              │                          apply event
   │                              │                              ↓
   │                              │                     rebuild local indexes
   │                              │                              ↓
   │                              │                       same memory context
```

The second device should be able to download the canonical records needed to reconstruct the user's brain even if the first device is offline.

## 8. Encryption model

Default requirements:

- TLS for transport
- encryption at rest on hosted storage
- OS keyring protection for local session credentials
- per-workspace authorization
- tenant-aware access checks
- explicit local-only source exclusions

For a future high-privacy mode, introduce client-side envelope encryption where the server stores ciphertext it cannot process. That mode is compatible with local-first retrieval but is not compatible with arbitrary server-side semantic retrieval over plaintext; the product must make that tradeoff explicit.

## 8A. Storing and processing policy pipeline

Enterprise governance must be enforced as a pipeline, not as a single database permission check. Every ingestion and processing path should carry the workspace policy context:

```text
Source / event
      ↓
Classification & secret scan
      ↓
Ownership / workspace resolution
      ↓
Policy gate
      ├── retention
      ├── residency
      ├── local-only / sync / hosted-processing
      ├── allowed connectors
      └── redaction / DLP rules
      ↓
Local processing
      ↓
Memory / embedding / graph generation
      ↓
Storage policy
      ├── local only
      ├── encrypted cloud replica
      └── enterprise managed storage
      ↓
Retrieval permission check
      ↓
Context packing
      ↓
Optional outbound-processing gate
```

Minimum enterprise policy controls:

- data classification (`personal`, `internal`, `confidential`, `restricted`, or tenant-defined classes)
- source allow/deny rules
- secret and credential detection
- local-only exclusions
- cloud-sync eligibility
- hosted-processing eligibility
- retention and legal-hold hooks
- deletion and export workflows
- workspace/tenant residency selection
- encryption and key-management policy
- auditability of sensitive reads, writes, sync and processing
- policy version attached to each processing decision

The policy engine must execute **before** cloud upload, hosted inference, cross-workspace retrieval, graph expansion and context export. A memory that is semantically relevant but policy-ineligible must be rejected before reranking can cause it to enter agent context.

The implementation should expose compliance-ready hooks rather than hard-code a single jurisdiction's legal rules. Jurisdiction-specific requirements can later be represented as policy bundles for the regions and enterprise customers ORBIT supports.

## 9. Sync scheduling

Provide:

- background sync when idle
- manual `Sync now`
- sync on app resume
- sync after important durable events
- retry with exponential backoff
- offline queue
- bandwidth controls
- pause sync per workspace

Do not run a network round trip on the memory recall hot path merely because a user is signed in.

## 10. Future mobile client

A future iOS/Android client should use the same canonical sync protocol. The mobile client does not need to reproduce the entire desktop storage engine on day one.

A mobile profile can initially:

1. hydrate durable memories and selected source summaries,
2. build a smaller local index,
3. perform lightweight local recall,
4. sync events back to the same workspace.

The desktop remains the full indexing environment, while the protocol stays device-neutral.

## Verification

- [ ] Desktop local-only mode works without account
- [ ] App launch presents Log In / Sign Up / Continue Local
- [ ] Signup and password entry occur only on the ORBIT auth website
- [ ] Existing authenticated sessions reopen without re-entering password
- [ ] Browser-first signup/login works
- [ ] PKCE/state validation blocks forged callback attempts
- [ ] Deep-link callback resumes an already-running app
- [ ] Fresh app instance can handle callback during startup
- [ ] Device registration is idempotent
- [ ] Device metadata is stored separately from memory/workspace content
- [ ] Raw MAC address is not required as an authenticator
- [ ] User can view/revoke registered devices
- [ ] Device revocation blocks subsequent cloud sync
- [ ] Two-device event replay produces equivalent logical memory
- [ ] Duplicate events are harmless
- [ ] Deletion tombstones prevent resurrection
- [ ] Local recall remains functional while offline
- [ ] Cloud sync never requires copying DB files
- [ ] Personal and enterprise workspace data remain isolated
- [ ] Policy engine blocks disallowed sync/processing before upload or hosted inference
- [ ] Retention/delete/export/audit controls are testable per workspace

---

## 4-Tier Implementation Layer Architecture

### 1. Frontend Tier: Identity & Subscription Management UX
- **Desktop Settings (`desktop/src/features/settings/ui/ProfileSettingsCard.tsx`)**:
  - Displays user profile, active subscription tier (`Local`, `Cloud Sync`, `Enterprise`), and connected devices.
  - "Log In / Sign Up" button opening external browser OAuth PKCE flow.
  - "Sign Out" with explicit "Keep Local Brain" or "Remove Local Data" prompt.

### 2. Desktop Backend Tier: Tauri Rust IPC & Deep-Link Handler
- **Location**: `desktop/src-tauri/src/deep_link.rs` and `desktop/src-tauri/src/identity.rs`
- Listens for `orbit://oauth/callback` scheme via `tauri-plugin-deep-link`.
- Exchanges single-use auth code for session tokens; stores secrets in OS keychain via `buzz-auth`.
- Dispatches incremental logical sync events from `~/.orbit/brain/sync/outbox/`.

### 3. Core Workspace Crates Tier (`crates/buzz-auth`, `crates/buzz-ws-client`, `crates/buzz-db`)
- `crates/buzz-auth`: PKCE verification, cryptographic token validation, and workspace entitlement gate.
- `crates/buzz-ws-client`: Secure WebSocket connection to team relay (`buzz-relay`).
- `crates/buzz-db`: Authoritative sync ledger in `orbit_sync_log`.

### 4. Packaging, Bundling & Container Tier
- **Zero Docker**: Native desktop client runs fully offline without containers or external auth services.
- **Relay Containers**: Centralized relay and auth server deployed via Docker Compose and Kubernetes for hosted mode.

