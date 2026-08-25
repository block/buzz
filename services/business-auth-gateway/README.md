# Business Auth Gateway

The V3.2 production boundary for Workbench identity binding, one-time Desktop
embed sessions, server-side Business sessions, logout, and append-only security
audit. It uses Authentik access tokens only on Workbench-to-Gateway calls and
never sends them to the Business iframe.

V4 adds internal Agent Delegation endpoints. They are disabled unless
`BUSINESS_AGENT_READ_ENABLED=true` and require the server-only
`BUSINESS_READ_SERVICE_CREDENTIAL`. Delegation tokens are opaque 256-bit
Base64URL values; only their SHA-256 hashes are stored. These endpoints are for
the trusted Agent Host and `business-read-mcp`, not browsers.

Run migrations with `business-auth-gateway --migrate-only`, then run the normal
binary as a least-privilege PostgreSQL role. Configuration is validated before
the listener starts. `/health/live` is process liveness and `/health/ready`
checks PostgreSQL.

Business IAM policy administration is deliberately absent from the HTTP
listener. Bootstrap and emergency changes use the separate
`business-iam-admin` binary with `BUSINESS_IAM_ADMIN_DATABASE_URL` and an
operator identity in `BUSINESS_IAM_ADMIN_ACTOR`. Its database role must be
different from `BUSINESS_AUTH_RUNTIME_DATABASE_ROLE`. Every successful change
is transactional and appends a `BUSINESS_IAM_ADMIN_MUTATION` audit event with
both the asserted operator and PostgreSQL `current_user`.

```bash
cargo run -p business-auth-gateway --bin business-iam-admin -- --help
```

`AUTHENTIK_ISSUER` must exactly match the discovery document's `issuer`,
including any trailing slash; the Gateway never normalizes this trust value.

Run one safe expiry sweep with `business-auth-gateway --cleanup-once`. Migration
0001 is intentionally forward-only because it creates the production authority;
rollback is application rollback plus database restore/snapshot, never an
automatic down migration that could discard identity or audit history.

The bootstrap route must be excluded from proxy query-string access logs. Its
response is `no-store`, creates a host-only `__Host-` HttpOnly Business cookie,
and redirects to a validated `/embed/` target. See the deployment example and
the Business Dock operations documentation.
