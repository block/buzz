# Deployment moderation dashboard

Buzz can expose a private, deployment-wide moderation dashboard from the existing
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
`BUZZ_ADMIN_HOST` activates the dashboard in the default `nip98` mode. Configure
the mode with `BUZZ_ADMIN_AUTH`, which accepts `nip98` (default when unset) or
`disabled`.

`BUZZ_ADMIN_TOKEN` is no longer recognised: token authentication was removed.
Setting it is ignored with a startup warning, so a stale token variable cannot
silently run a deployment on a removed auth path — remove it from the environment
and use `nip98` (or `disabled` behind a network boundary).

### NIP-98 mode (`BUZZ_ADMIN_AUTH=nip98`, default)

Every `/api/admin/v1` request must carry a NIP-98 HTTP Auth header containing a
signed kind-27235 event. The signer's pubkey is resolved against a two-tier
principal model — **Operator** or **Moderator** — that grants capabilities
accordingly.

```text
BUZZ_ADMIN_AUTH=nip98
RELAY_OPERATOR_PUBKEYS=<64-char hex pubkey>[,<64-char hex pubkey>...]
```

- A malformed `RELAY_OWNER_PUBKEY` alongside `nip98` is a startup error (see
  owner fallback below).
- `RELAY_OPERATOR_API_ORIGIN` is **not** required to run the admin console.
  That origin is only used by the community-provisioning endpoints
  (`POST /operator/communities`), which share the `RELAY_OPERATOR_PUBKEYS`
  allowlist. When the pubkeys are set but the origin is not, the relay boots
  with a `WARN` and provisioning requests fail closed at request time until the
  origin is set — the admin console is unaffected.

#### Auto-discovery via NIP-11

When `BUZZ_ADMIN_HOST` is set, the relay advertises the admin API origin in its
NIP-11 relay-information document under an optional `admin_api` field:

```json
{ "admin_api": "https://admin.example.com" }
```

The value is the canonical origin `scheme://host[:port]` (no path), with the
scheme derived by the same loopback rule as `u`-tag verification (`http` for
`localhost`/`127.x`/`[::1]`, else `https`). The field is omitted entirely when no
admin surface is configured. Clients (such as the desktop console) read this to
auto-discover the admin endpoint instead of requiring manual URL entry. IPv6
admin hosts must be bracketed (`[::1]`, `[::1]:3000`); an unbracketed literal is
a startup error.

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
returns `401` with `WWW-Authenticate: Nostr`. The dashboard's first-load probe
treats any non-`200` response as nip98 mode (only a `200` selects disabled
mode), so a credential is always required unless the relay explicitly serves the
unauthenticated surface.

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

**Last-operator protection.** When no config-backed operator is effective —
`RELAY_OPERATOR_PUBKEYS` is empty and owner fallback is inactive — the roster is
staffed entirely by DB `operator` rows. A demotion or delete that would remove
the final DB operator is rejected transactionally with `409 Conflict` ("add a
replacement operator first"); the row is left unchanged. The safe way to hand
off sole authority is the add-first transfer: `PUT` the replacement operator,
then demote or delete the outgoing one. Config-backed operators are unaffected —
while any config operator or owner fallback is effective, the DB roster may be
emptied freely because config still guarantees an operator.

### Disabled mode (`BUZZ_ADMIN_AUTH=disabled`)

Operators whose admin API is already protected at the network layer — for
example by a corporate VPN such as WARP+Okta — can disable request
authentication entirely:

```text
BUZZ_ADMIN_AUTH=disabled
```

Only the exact value `disabled` is accepted.

In this mode the relay logs a `WARN` on every startup:

```
BUZZ_ADMIN_AUTH=disabled — the admin API is unauthenticated; the operator has
asserted that access is controlled at the network layer
```

The `Host`/`Origin` checks remain active as defense-in-depth. The dashboard
detects that no credential is needed on first load (probe returns `200`) and
renders directly.

**This mode relies entirely on the operator's network controls.** If the admin
API is reachable by untrusted clients, the entire moderation and feedback dataset
is exposed. Use nip98 mode instead.

When using a reverse proxy in this mode, document the requirement and consider a
proxy-injected shared secret or signed identity header for additional assurance.

### Mode selection and error behaviour

`BUZZ_ADMIN_AUTH` accepts exactly `nip98` or `disabled`. Any other non-empty
value is a startup error (typo-proofing). `BUZZ_ADMIN_TOKEN` is no longer
recognised and its presence is ignored with a startup warning regardless of the
other variables:

| Combination | Result |
|---|---|
| `BUZZ_ADMIN_TOKEN` set (with or without `BUZZ_ADMIN_HOST`) | warn + ignore |
| `BUZZ_ADMIN_AUTH=nip98` with a malformed `RELAY_OWNER_PUBKEY` | startup error |
| `BUZZ_ADMIN_AUTH` junk value | startup error |
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
and a private ingress remain required: in nip98 mode the signed credential
travels in the `Authorization` header; in network-layer mode the VPN/firewall
boundary is the only access control.

When the UI runs in a separate pod, proxy `/api/admin/v1/*` to the relay while
preserving the admin `Host` header and (in nip98 mode) the client's
`Authorization` header. A `NetworkPolicy` grants the admin pod access to that
relay path.

## Operator migration

**Upgrading from a pre-auth release (Buzz prior to the introduction of this
`BUZZ_ADMIN_HOST` requirement):** any deployed relay with `BUZZ_ADMIN_HOST` set
boots in `nip98` mode after upgrade unless `BUZZ_ADMIN_AUTH=disabled` is set.
Choose the mode that fits your deployment:

- **Nostr principal model (default):** set `BUZZ_ADMIN_AUTH=nip98` (or leave it
  unset) and populate `RELAY_OPERATOR_PUBKEYS` with at least one operator pubkey
  (or rely on owner fallback if `RELAY_OWNER_PUBKEY` is already set) in your
  deploy config, then roll the new version. In a single config rollout, both set
  `BUZZ_ADMIN_AUTH=nip98` and add the operator pubkey(s) — never split these
  across separate rollouts, as a mode-flip without operators configured leaves
  no-one able to authenticate.
- **Network-layer mode (e.g. Block's `bb-public` behind WARP+Okta):** set
  `BUZZ_ADMIN_AUTH=disabled` in your deploy config, then roll the new version.

**Upgrading from a token-mode release:** token authentication was removed.
Remove `BUZZ_ADMIN_TOKEN` from the environment — leaving it set is ignored with
a startup warning — and adopt `nip98` (populate `RELAY_OPERATOR_PUBKEYS`) or
`disabled` before rolling the new version.

**Upgrading from the previous `BUZZ_ADMIN_INSECURE_NO_AUTH=true` variable:**
replace it with `BUZZ_ADMIN_AUTH=disabled`. Behavior is identical; the old
variable is no longer recognised.

Relays without `BUZZ_ADMIN_HOST` are completely unaffected, except that a
lingering `BUZZ_ADMIN_TOKEN` now logs a startup warning and must be removed.

Any non-browser client of `/api/admin/v1` (monitoring probes, scripts, cron
jobs) must sign each request with a NIP-98 `Authorization: Nostr` header after
the upgrade, unless the deployment runs in `disabled` mode. The dashboard handles
itself. If a reverse proxy strips or rewrites `Authorization` headers, the
dashboard breaks post-upgrade in nip98 mode — check proxy configuration before
rolling.

## Local development

For local review, run `just admin-seed` before `just admin`. `just admin`
defaults to `BUZZ_ADMIN_AUTH=disabled`, so the dashboard renders without a
credential. The seed command also uploads real image and diagnostic fixtures to
local MinIO. Feedback search and filters run over the bounded browser result
set. The feedback **status** control (`new`/`reviewed`/`archived`) is
server-backed: in `nip98` mode it `PATCH`es the relay and adopts the returned
status, so every operator sees the same state; in `disabled` mode it renders as
a read-only badge because the server rejects mutations.

## Routes

### Read routes (Operator and Moderator)

- `GET /api/admin/v1/probe`
- `GET /api/admin/v1/reports`
- `GET /api/admin/v1/reports/:id`
- `GET /api/admin/v1/feedback`
- `GET /api/admin/v1/feedback/:id`
- `GET /api/admin/v1/feedback/:id/attachments/:sha256`

### Action routes (Operator and Moderator)

- `POST /api/admin/v1/reports/:id/resolve`
  Body: `{"action": "delete|kick|ban|timeout|dismiss|escalate", "expirationSecs": <number>, "reason": "<string>", "requestId": "<uuid>"}`
  `expirationSecs` required for `timeout`, rejected for all others.
  Target/channel are always derived from server-owned report provenance.
  Response: `{"status": "<terminal>", "activeAction": <action | null>}`. Enforcement
  actions return the governing action record; `dismiss`/`escalate` return `null`.

  **`reason` is PUBLIC.** For `delete`/`kick`/`ban`/`timeout` the string is
  broadcast verbatim to the channel as the removal tombstone's public reason
  **and** sent verbatim to the affected user in a moderation DM. It is not
  sanitized, redacted, or mapped to a code. Never put private, internal, or
  report-derived context in `reason` — write only text that is safe for the
  room and the actioned user to read.
- `POST /api/admin/v1/reports/:id/reopen`
  Body: `{"requestId": "<uuid>", "reason": "<string>"}`
  Returns a terminal report (`resolved`, `dismissed`, or `escalated`) to `open` and
  records a durable `reopen` audit row. Idempotent on `requestId`: a retry after the
  report has been reopened (and possibly re-resolved) returns the same `200` without
  re-reopening. Returns `{"status": "open"}` on success, `409` if the report is not
  in a terminal state, `404` if it does not exist.
- `POST /api/admin/v1/reports/:id/cancel`
  Body: `{"actionId": "<uuid>"}`
  Cancels a pre-mutation `failed` enforcement action, returning the report to `open`.
  Cancel is the only recovery path for a failed action. `actionId` fences the cancel
  to exactly the action the client observed. Returns `{"status": "open", "activeAction":
  <cancelled action>}` — the embedded record is the last look at that action, since a
  subsequent detail read (report back to `open`) serves `activeAction: null`. Returns
  `409` if the action is not cancellable (already cancelled, superseded, or past the
  mutation point) — treat as "refresh detail".
- `PATCH /api/admin/v1/feedback/:id`
  Body: `{"status": "new|reviewed|archived"}`

### Staffing routes (Operator only)

- `GET /api/admin/v1/operators`
  Returns every effective principal with its `effectiveRole` and all
  contributing `sources` (`config`, `owner_fallback`, `db`).
- `PUT /api/admin/v1/operators/:pubkey`
  Body: `{"role": "operator|moderator"}`
  Returns `409` if the target is config-backed.
- `DELETE /api/admin/v1/operators/:pubkey`
  Returns `409` if the target is config-backed.

Report reads accept optional `communityId`, `status`, `reportType`, `targetKind`,
`after`, `before`, `scope`, and `limit` parameters. Limits are capped at 200. Feedback is
a bounded newest-first summary from the existing product-feedback repository.

### Escalation-scoped default

`GET /reports` with no `status` parameter returns only `escalated` reports — the
escalation backstop the operator queue is meant to hold per `VISION_MODERATION`,
where the severe class is the platform's to review rather than the community's.
An explicit `status=<open|resolved|dismissed|escalated>` filter is always honored
as given. Full visibility across every status stays available for platform-safety
and legal review via `scope=all`, which lists reports regardless of status;
`scope` accepts only `all` and is ignored when an explicit `status` is present.

Reports whose category is `illegal` bypass community triage entirely: they are
ingested with `status=escalated` rather than `open`, so the severe class reaches
the operator backstop without waiting for a community admin to forward it (per
`VISION_MODERATION`, that class was never the community's to hold). Every other
category still lands `open`. Auto-escalation only sets the queue status — it
records no moderator decision and stamps no resolver, so an auto-escalated report
is indistinguishable downstream from an admin-escalated one: the reopen route
returns it to `open` on the same terms, keyed only on status, never on how the
report became escalated.

## Feedback attachment boundary

Feedback attachment bytes are available only through the feedback-scoped read
route (`GET /api/admin/v1/feedback/:id/attachments/:sha256`, listed under Read
routes above).

The route uses the same credential requirement (NIP-98 event, or network-layer
boundary in disabled mode), private-ingress, exact admin `Host`, and same-origin
boundary as the JSON API. It is not a generic media endpoint. The relay loads
the feedback row, derives its community from server-owned provenance, verifies
that host resolution still maps to the row's `community_id`, and requires the
requested SHA-256 to match both the `x` field and source-community `/media/` URL
in that row's persisted `imeta` tag. It then reads the tenant-scoped media
sidecar before accessing the shared content-addressed blob. Unknown feedback,
unreferenced hashes, malformed paths, and cross-community substitutions all
collapse to `404`.

Product feedback is deployment-global operator evidence: when its source community
is purged, the row's `community_id` is severed to `NULL` (the row survives, its
provenance does not). List and detail reads still return such rows with
`communityId` and `communityHost` as `null`. Their attachments, however, were
purged with the tenant, so the attachment route fails closed to `404` for any
severed feedback — there is no tenant to bind and no tenant-scoped media to serve.

Only `GET` and `HEAD` are routed. Community `/media/*` reads always require
Blossom authorization and relay membership; the browser receives no reusable
signed URL. Responses are uncached, `nosniff`,
governed by a restrictive CSP, streamed from object storage, and non-previewable
content retains attachment disposition. Successful reads produce a structured
trace containing feedback ID, community ID, and attachment hash, but no feedback
body or attachment URL.

The human trust boundary is the chosen auth mode plus the private admin ingress.
Disabled mode provides no per-operator identity; anyone admitted to the dashboard
can read attachments for feedback records they can access. NIP-98 mode provides
per-operator attribution and individual revocability. Per-person identity in
disabled mode requires authenticated operator identity at ingress/application
level (for example an Okta-injected identity header).
