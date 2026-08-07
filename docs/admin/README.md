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

Only `GET` and `HEAD` are routed. Community `/media/*` reads always require
Blossom authorization and relay membership; the browser receives no reusable
signed URL. Responses are uncached, `nosniff`,
governed by a restrictive CSP, streamed from object storage, and non-previewable
content retains attachment disposition. Successful reads produce a structured
trace containing feedback ID, community ID, and attachment hash, but no feedback
body or attachment URL.

The human trust boundary remains the private admin ingress. VPN/source-IP
admission is not per-operator identity. Anyone admitted to the dashboard can
read attachments for feedback records they can access. Per-person attribution
or revocation requires authenticated operator identity at ingress/application
level; this endpoint deliberately does not claim to provide it.

## Authentication modes

Set `BUZZ_ADMIN_AUTH` to one of:

| Value | Behaviour |
|-------|-----------|
| `nip98` | Requires a `Authorization: Nostr <base64 NIP-98 event>` signed with a key listed in `BUZZ_ADMIN_PUBKEYS` |
| `token` | Requires `Authorization: Bearer <token>` where the token equals `BUZZ_ADMIN_TOKEN` |
| `disabled` | No credential required (development use only) |

The default is `token`.

## Desktop app

The Buzz desktop app ships a built-in admin console client. It does not require
a browser extension, separate web UI, or bearer token. It uses NIP-98
authentication (mode `nip98` only).

### Setup

1. In your relay config, set `BUZZ_ADMIN_AUTH=nip98`.
2. Add your identity pubkey to `BUZZ_ADMIN_PUBKEYS`:
   - Open **Settings → Admin console** in the Buzz desktop app.
   - Copy the hex pubkey shown in the denied-access message, or find it under
     **Settings → Profile**.
   - Paste it into your relay's `BUZZ_ADMIN_PUBKEYS` environment variable.
3. In **Settings → Admin console**, paste the value of `BUZZ_ADMIN_HOST` (e.g.
   `https://admin.yourrelay.example.com`) into the admin console URL field.
   - The URL must be an origin only (scheme + host + optional port). No path,
     query string, or fragment.
   - The host must match `BUZZ_ADMIN_HOST` exactly, including case. The relay
     compares byte-for-byte. Use lowercase.
   - `http://` is accepted only for `localhost`, `127.x.x.x`, and `[::1]`; all
     other hosts require `https://`.
4. Click **Save**. The app probes the origin and shows **Connected** when the
   current identity is on the allowlist.

### How it works

The desktop client signs a NIP-98 kind-27235 event for each request URL and
method using the app's own keypair (the same identity used for messaging). It
does not require a separate admin key.

The client uses a dedicated no-redirect HTTP client. A relay-issued redirect is
surfaced as an error rather than followed, so the `Authorization` header is
never forwarded to a different host. Every request URL is constructed natively
from a closed route enum — the webview cannot supply arbitrary paths.

Response sizes are bounded: JSON responses are capped at 50 MiB (sized for a
200-row report list where each note field may reach the 256 KiB event-content
limit). Attachment previews are capped at 10 MiB.

### Probe states

| State | Meaning |
|-------|---------|
| **Connected** | NIP-98 mode, current identity authorised |
| **Access denied** | NIP-98 mode, identity not in `BUZZ_ADMIN_PUBKEYS`; or clock skew > 60 s |
| **Bearer-token mode** | Relay requires a bearer token. Use the web console — the desktop app supports NIP-98 only |
| **Auth disabled** | `BUZZ_ADMIN_AUTH=disabled`. Accessible without a credential |
| **No admin API** | Origin reachable but `/api/admin/v1` not found. Check the URL matches `BUZZ_ADMIN_HOST` |
| **Network/intercepted** | Network or TLS error, DNS failure, or an SSO/VPN layer (e.g. Cloudflare Access) intercepting the host |

### Deployment note

> **The desktop client is NOT advertised as usable against `admin.buzz.xyz` until
> the Cloudflare Access carve-out follow-up resolves (separate arc).** Self-hosted
> deployments without Cloudflare Access interception are unaffected.
