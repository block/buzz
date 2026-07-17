# Read-only deployment moderation dashboard

Buzz can expose a private, deployment-wide read-only dashboard from the existing
relay process. It shows open moderation reports and recent product feedback. It
does not mutate moderation state.

The surface is absent unless `BUZZ_ADMIN_HOST` is configured. It has no
application-level or browser-managed authentication. Access is controlled by a
private ingress restricted to the operator VPN or approved source IPs.

Required configuration:

```text
BUZZ_ADMIN_HOST=admin.example.com
BUZZ_ADMIN_WEB_DIR=/srv/buzz/admin-web
```

The relay verifies the exact host. The relay service and admin host must not be
reachable through an unrestricted ingress or another network path that bypasses
the private ingress. Browser API requests are restricted to the admin origin,
independent of the relay's general CORS configuration. Requests and responses
are bounded and uncached.

When the UI runs in a separate pod, proxy `/api/admin/v1/*` to the relay while
preserving the admin `Host` header, and use a `NetworkPolicy` to restrict that
relay path to the admin pod.

Do not use a browser-supplied identity header as authentication: any client that
can reach the endpoint can forge it. If per-user identity or audit logs become a
requirement, configure an identity-aware proxy at the ingress and only trust
headers that it strips and injects itself.

Read routes:

- `GET /api/admin/v1/reports`
- `GET /api/admin/v1/reports/:id`
- `GET /api/admin/v1/feedback`
- `GET /api/admin/v1/feedback/:id`

Report reads accept optional `communityId`, `status`, `reportType`, `targetKind`,
`after`, `before`, and `limit` parameters. Limits are capped at 200. Feedback is
a bounded newest-first summary from the existing product-feedback repository.

There are deliberately no write routes, CSRF state, idempotency machinery,
mutation switches, synthetic Nostr commands, or schema changes. Community
moderation remains the only command plane.
