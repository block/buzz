# Feature 11 — Hosted / Enterprise Checklist

## Control plane
- [ ] Dedicated ORBIT authentication/account server
- [ ] Website signup/login/password recovery/email verification
- [ ] Desktop deep-link authorization and one-time code exchange
- [ ] OS-keychain session storage in desktop
- [ ] Web account signup/login exists outside the desktop UI
- [ ] Workspace/organization membership model implemented
- [ ] Subscription entitlement model implemented
- [ ] Device registry and revocation implemented
- [ ] Device security metadata separated from memory data
- [ ] Hardware fingerprinting is purpose-limited; raw MAC is not a sole authenticator

## Workspace ownership and isolation
- [ ] Personal and enterprise workspaces are separate ownership domains
- [ ] Organization/project ACLs enforced before retrieval and graph expansion
- [ ] Enterprise data cannot sync into personal workspace without explicit import policy
- [ ] Personal data is not visible to organization admins by default

## Hosted data plane
- [ ] PostgreSQL + pgvector schema implemented
- [ ] Object storage adapter implemented
- [ ] Sync event ingestion API implemented
- [ ] Server-side permission filters applied before retrieval
- [ ] Hosted worker queue implemented
- [ ] Idempotency keys enforced for all write events

## Local-first parity
- [ ] Shared logical memory schema used by local and hosted systems
- [ ] Shared retrieval/evaluation corpus exists
- [ ] Hosted processing can be disabled by workspace policy
- [ ] Desktop continues to use local indexes while online
- [ ] Cloud disconnect does not break local recall

## Storage / processing governance
- [ ] Data classification attached to enterprise records
- [ ] Workspace policy attached to ingestion, memory, embedding and sync jobs
- [ ] Local-only source exclusions enforced
- [ ] Cloud-sync eligibility enforced before outbound transfer
- [ ] Hosted-processing eligibility enforced before model calls
- [ ] Retention / legal-hold / deletion / export hooks implemented
- [ ] Data residency policy is represented in workspace configuration
- [ ] Approved model/provider allow-list enforced
- [ ] Policy version recorded with processing decisions

## Enterprise security
- [ ] Tenant isolation tests pass
- [ ] Workspace/project ACL tests pass
- [ ] Encryption in transit and at rest enabled
- [ ] Audit events for security-sensitive operations
- [ ] Data export and deletion workflows pass
- [ ] Retention policies are enforceable

