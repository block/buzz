# Feature 13 — Auth Server & Data Governance Checklist

## Auth server
- [ ] Dedicated hosted authentication service
- [ ] Email/password signup
- [ ] Email verification
- [ ] Password recovery
- [ ] Session/token lifecycle
- [ ] Desktop authorization code + PKCE
- [ ] Deep-link callback handling

## Device identity
- [ ] Random installation `device_id`
- [ ] Device key pair
- [ ] Device registration/revocation
- [ ] Platform/app metadata
- [ ] Purpose-limited derived hardware fingerprint
- [ ] No raw MAC as sole authenticator

## Workspace governance
- [ ] Personal versus enterprise ownership domains
- [ ] Tenant/workspace/project ACLs
- [ ] Data classification
- [ ] Local-only exclusions
- [ ] Sync policy
- [ ] Hosted-processing policy
- [ ] Model/provider allow-list
- [ ] Residency configuration

## Lifecycle & compliance-ready controls
- [ ] Retention policies
- [ ] Deletion/tombstones
- [ ] Data export
- [ ] Legal-hold extension point
- [ ] Security audit events
- [ ] Optional analytics controls
- [ ] Sensitive-content redaction before telemetry
- [ ] Policy version recorded with processing decisions
