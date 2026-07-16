# Read-only deployment moderation dashboard

Buzz can expose a private, deployment-wide read-only dashboard from the existing
relay process. It shows open moderation reports and recent product feedback. It
does not mutate moderation state.

The surface is absent unless `BUZZ_ADMIN_HOST` is configured. Trusted ingress
must route only that exact host, strip client-provided copies of the reviewer
header, authenticate the human, and inject the reviewer identity.

Required configuration:

```text
BUZZ_ADMIN_HOST=admin.example.com
BUZZ_ADMIN_REVIEWER_HEADER=x-authenticated-user
BUZZ_ADMIN_REVIEWERS=reviewer@example.com
BUZZ_ADMIN_WEB_DIR=/srv/buzz/admin-web
```

The relay verifies the exact host and requires the injected reviewer to appear in
the application allowlist. Requests and responses are bounded and uncached.

Read routes:

- `GET /api/admin/v1/reports`
- `GET /api/admin/v1/reports/:id`
- `GET /api/admin/v1/feedback`

Report reads accept optional `communityId`, `status`, `reportType`, `targetKind`,
`after`, `before`, and `limit` parameters. Limits are capped at 200. Feedback is
a bounded newest-first summary from the existing product-feedback repository.

There are deliberately no write routes, CSRF state, idempotency machinery,
mutation switches, synthetic Nostr commands, or schema changes. Community
moderation remains the only command plane.
