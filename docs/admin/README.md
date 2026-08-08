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

Plus one of the authentication modes below.

## Authentication

The admin API requires explicit authentication configuration. Setting only
`BUZZ_ADMIN_HOST` is a startup error — there is no insecure default. Configure
the mode with `BUZZ_ADMIN_AUTH` (defaults to `token` when unset).

### Token mode (`BUZZ_ADMIN_AUTH=token`, default)

Every `/api/admin/v1` request must carry the operator token as a bearer
credential.

```text
BUZZ_ADMIN_AUTH=token    # optional — this is the default
BUZZ_ADMIN_TOKEN=<64 hex characters>
```

The relay fails closed if `BUZZ_ADMIN_TOKEN` is missing or invalid when
`BUZZ_ADMIN_HOST` is set:

- `BUZZ_ADMIN_TOKEN` must be exactly 64 hexadecimal characters (32 bytes).
  Surrounding whitespace is trimmed; anything else — empty, non-hex, wrong
  length, non-Unicode — is a startup error.
- `BUZZ_ADMIN_TOKEN` set without `BUZZ_ADMIN_HOST` is ignored: the admin surface
  stays absent and the relay logs a warning at startup.

Generate a token once per deployment and store it with your other secrets:

```bash
openssl rand -hex 32
```

Call the API with it:

```bash
curl -H "Host: admin.example.com" \
     -H "Authorization: Bearer $BUZZ_ADMIN_TOKEN" \
     https://admin.example.com/api/admin/v1/reports
```

A missing, malformed, duplicated, or incorrect credential returns `401` with
`WWW-Authenticate: Bearer` and reveals nothing about the expected `Host`. The
scheme is matched case-insensitively per RFC 9110, and the credential is
compared in constant time. The token never appears in URLs, logs, or traces.

The dashboard probes for auth mode on first load: if the relay returns `401` to
an unauthenticated request with `WWW-Authenticate: Bearer`, the dashboard prompts
for the token and keeps it in `sessionStorage` for that browser session; a
rejected token is discarded and re-prompted. Attachment bytes are fetched through
the authenticated API and rendered from object URLs, because `<img src>` and
`<a href>` cannot carry an `Authorization` header.

In token mode, each new browser session issues one unauthenticated probe that
returns `401` by design before the token is entered. If you alert on admin-API
`401`s, exclude these single-probe sequences (one `401` immediately followed by
authenticated requests from the same session) to avoid false positives.

The shared token authenticates the deployment operator role, not a person. It
carries no per-operator identity, attribution, or individual revocation —
rotating it revokes access for everyone at once.

### NIP-98 mode (`BUZZ_ADMIN_AUTH=nip98`)

Every `/api/admin/v1` request must carry a NIP-98 HTTP Auth header containing a
signed kind-27235 event. The signer's pubkey is resolved against a two-tier
principal model — **Operator** or **Moderator** — that grants capabilities
accordingly.

```text
BUZZ_ADMIN_AUTH=nip98
RELAY_OPERATOR_PUBKEYS=<64-char hex pubkey>[,<64-char hex pubkey>...]
```

- `BUZZ_ADMIN_TOKEN` set alongside `nip98` is a startup error (ambiguous intent).
- A malformed `RELAY_OWNER_PUBKEY` alongside `nip98` is a startup error (see
  owner fallback below).

Each request requires:

```http
Authorization: Nostr <base64(JSON event)>
```

The event must be kind 27235, have a `u` tag matching the exact request URL
(including any query string, e.g.
`https://admin.example.com/api/admin/v1/reports?status=open`), a `method` tag
matching the HTTP method, a valid Schnorr signature, and a `created_at` within
±60 seconds of now. Body-bearing mutations (`POST`, `PUT`, `PATCH`) additionally
require a `payload` tag containing the SHA-256 hex digest of the raw request
body. A deployment-scoped replay guard rejects reused event IDs; Redis failure
fails closed.

Any auth failure (bad event, bad signature, expired, replay, unrecognised pubkey,
missing/incorrect `payload` tag on mutations, duplicate `Authorization` header)
returns `401` with `WWW-Authenticate: Nostr`. The dashboard uses this header to
discover the auth mode on first load.

The dashboard requires a NIP-07 browser extension (such as
[nos2x](https://github.com/fiatjaf/nos2x) or [Alby](https://getalby.com)). If
no extension is detected, the dashboard shows an installation screen. Once an
extension is present, each API request is automatically signed with the
principal's nostr key without any prompts.

#### Principal model: Operator and Moderator

The signer's pubkey is resolved to a principal with an effective role and a
source that describes how the grant was established:

| Resolution order | Role | Source |
|---|---|---|
| Pubkey is in `RELAY_OPERATOR_PUBKEYS` (config) | Operator | `config` |
| Pubkey equals `RELAY_OWNER_PUBKEY` **and** `RELAY_OPERATOR_PUBKEYS` is empty | Operator | `owner_fallback` |
| Pubkey has a row in the `relay_operators` DB table | Operator or Moderator | `db` |
| No grant found | — | 403 |

Config always outranks DB: a DB row for a config-backed pubkey is ignored and
never demotes the config grant. `None` never falls through as a role.

**Owner fallback** is an implicit Operator grant for self-hosters that do not yet
have an operator configured. It activates only when the configured
`RELAY_OPERATOR_PUBKEYS` list is empty, and it is evaluated from config at
request time — staffing the roster cannot make it flap. Once any pubkey is added
to `RELAY_OPERATOR_PUBKEYS`, the fallback deactivates. A malformed
`RELAY_OWNER_PUBKEY` is a startup error (not warn-and-ignore): once the owner key
can be a break-glass root, silently discarding it would be a lockout.

#### Capabilities by role

| Capability | Operator | Moderator |
|---|---|---|
| Read reports, feedback, attachment bytes | ✓ | ✓ |
| Resolve reports (dismiss, escalate, delete, kick, ban, timeout) | ✓ | ✓ |
| Update feedback status | ✓ | ✓ |
| Manage operator roster (`GET/PUT/DELETE /operators`) | ✓ | ✗ |

Capability checks are server-authoritative; the desktop console hides Staffing
tab controls for Moderators as a UX convenience only.

#### Roster management

Operators manage the roster via the staffing endpoints (`GET/PUT/DELETE
/operators/{pubkey}`). Staffing operations are only available in `nip98` mode.

Config-backed pubkeys (`RELAY_OPERATOR_PUBKEYS`, owner fallback) cannot be
modified through the API — `PUT` or `DELETE` against a config-backed pubkey
returns `409 Conflict`. A DB moderator row for a config-backed Operator pubkey is
ignored; it never demotes the config grant.

`GET /operators` returns every effective principal with its `effectiveRole` and
all contributing `sources` (`config`, `owner_fallback`, `db`).

### Disabled mode (`BUZZ_ADMIN_AUTH=disabled`)

Operators whose admin API is already protected at the network layer — for
example by a corporate VPN such as WARP+Okta — can disable bearer authentication
entirely:

```text
BUZZ_ADMIN_AUTH=disabled
```

Only the exact value `disabled` is accepted. `BUZZ_ADMIN_TOKEN` and
`BUZZ_ADMIN_AUTH=disabled` set at the same time is a startup error (ambiguous
intent).

In this mode the relay logs a `WARN` on every startup:

```
BUZZ_ADMIN_AUTH=disabled — the admin API is unauthenticated; the operator has
asserted that access is controlled at the network layer
```

The `Host`/`Origin` checks remain active as defense-in-depth. The dashboard
detects that no credential is needed on first load (probe returns `200`) and
skips any auth prompt, rendering the dashboard directly.

**This mode relies entirely on the operator's network controls.** If the admin
API is reachable by untrusted clients, the entire moderation and feedback dataset
is exposed. Use token or nip98 mode instead.

When using a reverse proxy in this mode, document the requirement and consider a
proxy-injected shared secret or signed identity header for additional assurance.

### Mode selection and error behaviour

`BUZZ_ADMIN_AUTH` accepts exactly `token`, `disabled`, or `nip98`. Any other
non-empty value is a startup error (typo-proofing). Conflicting combinations also
abort startup:

| Combination | Result |
|---|---|
| `BUZZ_ADMIN_AUTH=token` without `BUZZ_ADMIN_TOKEN` | startup error |
| `BUZZ_ADMIN_AUTH=disabled` + `BUZZ_ADMIN_TOKEN` | startup error |
| `BUZZ_ADMIN_AUTH=nip98` + `BUZZ_ADMIN_TOKEN` | startup error |
| `BUZZ_ADMIN_AUTH=nip98` with a malformed `RELAY_OWNER_PUBKEY` | startup error |
| `BUZZ_ADMIN_AUTH` junk value | startup error |
| `BUZZ_ADMIN_TOKEN` without `BUZZ_ADMIN_HOST` | warn + ignore |
| `BUZZ_ADMIN_AUTH` without `BUZZ_ADMIN_HOST` | warn + ignore |

## Content Security Policy

Every admin-host response that carries the dashboard itself — the SPA document
on each admin route, the hashed `/assets/*` bundle, and admin-host `404`s — is
served with a Content Security Policy response header, `ADMIN_CSP` in
`crates/buzz-relay/src/router.rs`:

```text
default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' blob:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'
```

It blocks inline and third-party script and restricts subresource and request
destinations to the same origin, which closes the direct paths an injected
script would use to exfiltrate credentials or data. It does not constrain
top-level navigation, so it is a containment layer, not a substitute for
keeping script off the origin. `blob:` is permitted for images only, for
attachment previews. It is a response header rather than a `<meta>` tag because
`frame-ancestors` is ignored in meta — that directive is the dashboard's
authoritative frame protection, superseding the `X-Frame-Options: DENY` the JSON
API sends. The policy applies to the admin host only; the public web bundle
keeps its own headers.

The exact admin `Host` and matching browser `Origin` are still required in both
auth modes, but they are defense-in-depth, not the primary access control. HTTPS
and a private ingress remain required: in token mode the token is a bearer
credential in transit; in network-layer mode the VPN/firewall boundary is the
only access control.

When the UI runs in a separate pod, proxy `/api/admin/v1/*` to the relay while
preserving the admin `Host` header and (in token mode) the client's
`Authorization` header. A `NetworkPolicy` grants the admin pod access to that
relay path.

## Operator migration

**Upgrading from a pre-auth release (Buzz prior to the introduction of this
`BUZZ_ADMIN_HOST` requirement):** any deployed relay with `BUZZ_ADMIN_HOST` set
refuses to start after upgrade unless `BUZZ_ADMIN_AUTH` (or `BUZZ_ADMIN_TOKEN`
for the default token mode) is also set. Choose the mode that fits your
deployment:

- **Token mode:** mint a token with `openssl rand -hex 32`, set `BUZZ_ADMIN_TOKEN`
  in your deploy config, then roll the new version.
- **Network-layer mode (e.g. Block's `bb-public` behind WARP+Okta):** set
  `BUZZ_ADMIN_AUTH=disabled` in your deploy config, then roll the new version.
- **Nostr principal model:** set `BUZZ_ADMIN_AUTH=nip98` and populate
  `RELAY_OPERATOR_PUBKEYS` with at least one operator pubkey (or rely on owner
  fallback if `RELAY_OWNER_PUBKEY` is already set) in your deploy config, then
  roll the new version. In a single config rollout, both set `BUZZ_ADMIN_AUTH=nip98`
  and add the operator pubkey(s) — never split these across separate rollouts, as
  a mode-flip without operators configured leaves no-one able to authenticate.

**Upgrading from the previous `BUZZ_ADMIN_INSECURE_NO_AUTH=true` variable:**
replace it with `BUZZ_ADMIN_AUTH=disabled`. Behavior is identical; the old
variable is no longer recognised.

Relays without `BUZZ_ADMIN_HOST` are completely unaffected.

Any non-browser client of `/api/admin/v1` using the token mode (monitoring
probes, scripts, cron jobs) must add `Authorization: Bearer` to their requests
after the upgrade. The dashboard handles itself. If a reverse proxy strips or
rewrites `Authorization` headers, the dashboard breaks post-upgrade even with the
token set — check proxy configuration before rolling.

## Local development

For local review, run `just admin-seed` before `just admin`. `just admin` mints a
throwaway token for that run and prints it — paste it into the dashboard prompt.
The seed command also uploads real image and diagnostic fixtures to local MinIO.
Feedback search and filters run over the bounded browser result set; the
**Acted on** checkbox is stored in that browser's local storage.

## Routes

### Read routes (Operator and Moderator)

- `GET /api/admin/v1/probe`
- `GET /api/admin/v1/reports`
- `GET /api/admin/v1/reports/:id`
- `GET /api/admin/v1/feedback`
- `GET /api/admin/v1/feedback/:id`
- `GET /api/admin/v1/feedback/:id/attachments/:sha256`
- `GET /api/admin/v1/operators`

### Action routes (Operator and Moderator, nip98 only)

- `POST /api/admin/v1/reports/:id/resolve`
  Body: `{"action": "delete|kick|ban|timeout|dismiss|escalate", "expirationSecs": <number>, "reason": "<string>", "requestId": "<uuid>"}`
  `expirationSecs` required for `timeout`, rejected for all others.
  Target/channel are always derived from server-owned report provenance.
- `PATCH /api/admin/v1/feedback/:id`
  Body: `{"status": "new|reviewed|archived"}`

### Staffing routes (Operator only, nip98 only)

- `PUT /api/admin/v1/operators/:pubkey`
  Body: `{"role": "operator|moderator"}`
  Returns `409` if the target is config-backed.
- `DELETE /api/admin/v1/operators/:pubkey`
  Returns `409` if the target is config-backed.

Report reads accept optional `communityId`, `status`, `reportType`, `targetKind`,
`after`, `before`, and `limit` parameters. Limits are capped at 200. Feedback is
a bounded newest-first summary from the existing product-feedback repository.

## Feedback attachment boundary

Feedback attachment bytes are available only through the feedback-scoped read
route (`GET /api/admin/v1/feedback/:id/attachments/:sha256`, listed under Read
routes above).

The route uses the same credential requirement (bearer token, NIP-98 event, or
network-layer boundary in disabled mode), private-ingress, exact admin `Host`, and same-origin
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

The human trust boundary is the chosen auth mode plus the private admin ingress.
Token mode and disabled mode provide no per-operator identity; anyone admitted
to the dashboard can read attachments for feedback records they can access. NIP-98
mode provides per-operator attribution and individual revocability. Per-person
identity in token or disabled mode requires authenticated operator identity at
ingress/application level (for example an Okta-injected identity header).
