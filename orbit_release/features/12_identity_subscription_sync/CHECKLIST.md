# Feature 12 — Identity, Subscription & Multi-Device Sync Checklist

## Authentication
- [ ] First launch presents `Log In`, `Sign Up`, and `Continue Local`
- [ ] Dedicated ORBIT auth server owns account registration and password handling
- [ ] Login session is restored from OS keychain on subsequent launches
- [ ] No plaintext password is stored in the desktop client
- [ ] Website-first signup/login
- [ ] OAuth public client + PKCE
- [ ] HTTPS app/universal link registration
- [ ] Reverse-domain private-use scheme fallback
- [ ] State and nonce validation
- [ ] One-time authorization code exchange
- [ ] OS keyring session storage
- [ ] One-time deep-link authorization result
- [ ] Desktop session stores account/workspace identifiers only, not brain content

## Device identity & security
- [ ] Random per-installation `device_id`
- [ ] Device public/private key pair
- [ ] Device registration/revocation
- [ ] Platform/app-version metadata
- [ ] Purpose-limited derived hardware fingerprint only where needed
- [ ] Raw MAC address not used as sole authentication factor
- [ ] Security telemetry separated from memory/workspace data

## Subscription
- [ ] Entitlement API
- [ ] Local cached entitlement state
- [ ] Cloud sync gated by entitlement
- [ ] Local brain never depends on subscription connectivity

## Workspace separation & policy
- [ ] Personal workspace and enterprise workspace are distinct ownership domains
- [ ] Enterprise project/workspace ACLs enforced before retrieval/graph expansion
- [ ] Data classification attached to ingest records
- [ ] Retention/deletion/export policy attached to workspace
- [ ] Local-only / cloud-sync / hosted-processing policy evaluated before outbound processing

## Sync
- [ ] Logical event/mutation protocol
- [ ] Per-device outbox/inbox
- [ ] Sync cursor
- [ ] Idempotent replay
- [ ] Conflict-resolution policies by memory class
- [ ] Tombstones for deletions
- [ ] Two-device integration test
- [ ] Offline/online transition test

## Future mobile
- [ ] Device-neutral sync protocol
- [ ] Smaller mobile retrieval/index profile specified
- [ ] Mobile can hydrate and recall synced durable memories
