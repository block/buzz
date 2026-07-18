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

## Feedback attachment boundary

Feedback attachment bytes are available only through the feedback-scoped read
route:

- `GET /api/admin/v1/feedback/:id/attachments/:sha256`

The route uses the same private-ingress, exact admin `Host`, and same-origin
boundary as the JSON API. It is not a generic media endpoint. The relay loads
the feedback row, derives its community from server-owned provenance, verifies
that host resolution still maps to the row's `community_id`, and requires the
requested SHA-256 to match both the `x` field and source-community `/media/` URL
in that row's persisted `imeta` tag. It then reads the tenant-scoped media
sidecar before accessing the shared content-addressed blob. Unknown feedback,
unreferenced hashes, malformed paths, and cross-community substitutions all
collapse to `404`.

Only `GET` and `HEAD` are routed. Existing community `/media/*` authorization is
unchanged, including `BUZZ_REQUIRE_MEDIA_GET_AUTH`; the browser receives no
Blossom credential or reusable signed URL. Responses are uncached, `nosniff`,
governed by a restrictive CSP, streamed from object storage, and non-previewable
content retains attachment disposition. Successful reads produce a structured
trace containing feedback ID, community ID, and attachment hash, but no feedback
body or attachment URL.

The human trust boundary remains the private admin ingress. WARP/source-IP
admission is not per-operator identity. Anyone admitted to the dashboard can
read attachments for feedback records they can access. Per-person attribution
or revocation requires authenticated operator identity at ingress/application
level; this endpoint deliberately does not claim to provide it.
