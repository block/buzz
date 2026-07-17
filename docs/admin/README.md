# Read-only deployment moderation dashboard

Buzz can expose a private, deployment-wide read-only dashboard from the existing
relay process. It shows open moderation reports and recent product feedback.

Configure `BUZZ_ADMIN_HOST` to activate the dashboard. A private ingress limits
access to the operator VPN or approved source IPs.

Required configuration:

```text
BUZZ_ADMIN_HOST=admin.example.com
BUZZ_ADMIN_WEB_DIR=/srv/buzz/admin-web
```

The relay requires the configured admin host and matching browser origin.
Requests and responses are bounded and uncached. The deployment routes admin
traffic through the private ingress.

When the UI runs in a separate pod, proxy `/api/admin/v1/*` to the relay while
preserving the admin `Host` header. A `NetworkPolicy` grants the admin pod access
to that relay path.

Read routes:

- `GET /api/admin/v1/reports`
- `GET /api/admin/v1/reports/:id`
- `GET /api/admin/v1/feedback`
- `GET /api/admin/v1/feedback/:id`

Report reads accept optional `communityId`, `status`, `reportType`, `targetKind`,
`after`, `before`, and `limit` parameters. Limits are capped at 200. Feedback is
a bounded newest-first summary from the existing product-feedback repository.

For local review, run `just admin-seed` before `just admin`. The seed command
also uploads real image and diagnostic fixtures to local MinIO. Feedback search
and filters run over the bounded browser result set; the **Acted on** checkbox is
stored in that browser's local storage.
