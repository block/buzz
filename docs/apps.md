# Community Apps

Community Apps receive authenticated HTTP callbacks and post repository-routed
notifications into a project channel. An App is a community-local integration
actor attested by the relay. It is not a user, agent, workflow, or channel
member. It cannot choose a community, channel, project, author, kind, or tags.

Owners and admins manage Apps through signed Nostr commands (kind `9038`) and
discover them through relay-signed metadata (kind `39008`). There is no App-list
HTTP endpoint. The only App HTTP surface is the callback:

```text
POST /hooks/apps/{app_id}
```

Existing workflow webhooks at `POST /hooks/{workflow_id}` are unchanged.

## Prerequisites

- You must be a **current community owner or admin**. Members and unauthenticated
  callers cannot create, update, rotate, enable, or disable Apps. The relay
  enforces this even if a client hides the controls.
- The destination is **server-derived**. The callback names a repository; Buzz
  resolves exactly one claim-valid project and that project's live
  `buzz-channel` in the same community. Create the repository, project, and
  channel first.
- Desktop App management lives under **Community Settings → Apps**. Mobile has
  no App management UI in this version.
- There is **no delete**. Disable an App to stop new callbacks while keeping
  metadata queryable so historical messages stay attributable.

## Manage Apps

### CLI

Requires `BUZZ_RELAY_URL` and an owner/admin `BUZZ_PRIVATE_KEY`. Live command
walkthrough: [`crates/buzz-cli/TESTING.md`](../crates/buzz-cli/TESTING.md#69a-community-apps).

```bash
buzz apps list
buzz --format compact apps list
buzz apps create --name Buildkite --description "CI notifications"
# prints app_id, callback_url, webhook_secret once
buzz apps update --app <uuid> --name "Buildkite Prod" --clear-description
buzz apps rotate-secret --app <uuid>
buzz apps enable --app <uuid>
buzz apps disable --app <uuid>
```

`list` queries NIP-11 `self` plus verified kind `39008` events. Compact list
fields are `app_id`, `name`, `status`, and `callback_url` — never a secret.

Create and rotate print the raw secret only in the successful JSON:

```json
{
  "app_id": "6eb31227-8ed2-42ec-9024-863497cbeed2",
  "callback_url": "https://relay.example.com/hooks/apps/6eb31227-8ed2-42ec-9024-863497cbeed2",
  "webhook_secret": "<redacted>"
}
```

Store the secret immediately. A lost create or rotate response is recovered
only by rotating again. A duplicate command event is accepted without a second
mutation and returns no secret (CLI exit `5`); run
`buzz apps rotate-secret --app <uuid>`.

Limits: name is trimmed, non-empty, at most 128 Unicode scalar values;
description at most 2,048 Unicode scalar values; icon URL at most 4,096 UTF-8
bytes and must be `http://`, `https://`, or `data:image/`. App names are not
unique. The App UUID is the stable identifier.

### Desktop

Open **Community Settings → Apps**. The section is shown only when the current
membership snapshot says you are an owner or admin. While membership is pending,
Desktop shows “Checking App management permissions…”. If membership fails or the
viewer is not an owner/admin, it shows “Apps settings are unavailable.” and
fails closed. The relay remains the authorization boundary.

Create App asks for a name and optional description and icon. After a successful
create or rotate, a one-time dialog shows the callback URL and
`X-Webhook-Secret` with separate copy controls:

> This secret is shown only once. Copy it now. Closing this dialog removes it
> from the app.

Rotate secret and Disable require confirmation. Rotate immediately invalidates
the previous secret. Disable preserves metadata and delivery history. This
version has no delete action.

The list shows icon, name, description, Active/Disabled status, callback URL
derived from the current relay HTTP URL and App UUID, and last-updated time.

## One-time secret

Buzz generates 32 cryptographically random bytes (256 bits) and encodes them as
unpadded base64url. PostgreSQL stores only `SHA-256` of those bytes.

The raw secret appears only in the successful create or rotate response (CLI
JSON or Desktop one-time dialog). It must not appear in metadata events, later
reads, logs, traces, audit details, caches, URLs, query strings, or screenshots.

## Callback contract

### URL and headers

```text
POST /hooks/apps/{app_id}
Host: <community-host>
X-Webhook-Secret: <secret>
Content-Type: application/json
```

- `{app_id}` is a canonical lowercase hyphenated UUID.
- `Host` selects the community **before** App lookup.
- Authenticate with `X-Webhook-Secret` only. A secret in the query string never
  authenticates.
- `X-Webhook-Signature-V2` and `X-Webhook-Timestamp` are ignored for authority
  in this version.
- Unknown communities, missing Apps, cross-community App IDs, and disabled Apps
  all return the same `404` with code `app_not_found`.

### JSON body

```json
{
  "idempotency_key": "stable-provider-event-id",
  "repository_name": "harness-service",
  "event_type": "workflow.run.completed",
  "content": "✅ completed speckit-feature for harness-service",
  "metadata": {}
}
```

| Field | Required | Contract |
| --- | --- | --- |
| `idempotency_key` | yes | Non-empty string, at most 512 UTF-8 bytes. Hashed as given; not trimmed. Dropped after hashing and never stored. |
| `channel_name` | no | Non-empty string, at most 255 UTF-8 bytes. When present, routes directly to the live stream channel with this exact name and `repository_name` is ignored. See [Routing](#routing). |
| `repository_name` | when `channel_name` is absent | Non-empty string, at most 512 UTF-8 bytes. Passed unchanged to the exact-tier route resolver. |
| `event_type` | yes | Lowercase ASCII matching `[a-z0-9][a-z0-9._:-]{0,63}`. |
| `content` | yes | Non-empty UTF-8. The 64 KiB HTTP body limit is the effective bound. Published as the kind `9` message content. |
| `metadata` | no | JSON object. Opaque to routing and attribution. Defaults to absent, which is **not** the same payload as `"metadata": {}`. |

Allowed top-level keys are exactly those six. Unknown keys and authority-like
fields are rejected as `400` / `invalid_callback`. Reserved examples:

`channel_id`, `project_id`, `community_id`, `app_id`, `pubkey`, `author`,
`tags`, `kind`.

Those fields never affect authorization or routing.

### Size limits

| Limit | Value |
| --- | --- |
| HTTP body | 64 KiB (`65_536` bytes) |
| `metadata` serialized | 32 KiB |
| `metadata` JSON nesting | 16 levels |
| `metadata` nodes (object members + array elements) | 1,024 |

Oversized bodies are `400` / `invalid_callback`. Invalid `metadata` bounds are
`422` / `invalid_control_fields` and create no ledger row.

### Example

Keep the secret in an environment variable. Never put it in the URL.

```bash
export BUZZ_APP_WEBHOOK_SECRET='<redacted>'

curl -sS -X POST "https://relay.example.com/hooks/apps/6eb31227-8ed2-42ec-9024-863497cbeed2" \
  -H "Host: relay.example.com" \
  -H "Content-Type: application/json" \
  -H "X-Webhook-Secret: ${BUZZ_APP_WEBHOOK_SECRET}" \
  --data @- <<'EOF'
{
  "idempotency_key": "provider-event-0001",
  "repository_name": "harness-service",
  "event_type": "workflow.run.completed",
  "content": "✅ completed speckit-feature for harness-service"
}
EOF
```

Success (new or identical replay):

```json
{
  "delivery_id": "11111111-2222-3333-4444-555555555555",
  "event_id": "<64-hex>",
  "status": "delivered",
  "replayed": false
}
```

Errors use `{"error": "<redacted message>", "code": "<stable_code>"}`.

## Routing

### Named channel

When the body has `channel_name`, Buzz looks for a channel in the same
community with exactly that name (case-sensitive, no trimming) that is a
`stream` channel, not archived, and not deleted. DM, forum, and workflow
channels never match. Private channels match too; the App does not need
channel membership.

Zero matches → `channel_missing`. Multiple matches → `channel_ambiguous`.
Buzz never falls back to `repository_name` when the named channel does not
resolve. The published message has no repository or project `a` tags.

### Repository

When `channel_name` is absent, Buzz routes by repository:

The App path reuses the shared repository → project → channel resolver with an
**empty alias map**. Comparison is case-sensitive and does not trim or
normalize Unicode.

Repository identity, first matching tier wins only when it identifies one
distinct repository coordinate:

1. Exact repository `d` tag.
2. Exact clone URL basename.
3. Exact repository display name.

Zero matches → `repository_missing`. Multiple distinct matches →
`repository_ambiguous`.

Buzz then enumerates the latest live listed projects that contain that
coordinate. Only a project signed by the repository owner or a current
maintainer is claim-valid. Exactly one claim-valid project must exist
(`project_missing` / `project_ambiguous` otherwise).

The selected project's single valid `buzz-channel` UUID must exist, be live,
not archived, and belong to the same community (`project_channel_invalid`
otherwise). The App does not need channel membership.

Buzz never falls back to another project, an App default channel, or a
workflow home channel.

## Idempotency and retries

Admission is serialized per `(community_id, app_id, SHA-256(idempotency_key))`.
The canonical payload hash is SHA-256 of the JSON object after removing only
`idempotency_key`. Object key order does not affect equality. Array order,
field presence, scalar type, and scalar value do. Omitted `metadata` and
`"metadata": {}` are different payloads.

| Situation | Result |
| --- | --- |
| New key, valid route | One kind `9` message and one delivered ledger row. HTTP `202`, `replayed: false`. |
| Same key, same payload | Existing result, no second message. HTTP `202`, `replayed: true`. |
| Same key, different payload | HTTP `409` / `idempotency_conflict`. Record unchanged. |
| Concurrent identical callbacks | One record and one message. |
| Deterministic route rejection | One rejected row; retries replay the same `422` code and redacted message. |
| Auth failure, missing usable key, invalid JSON/shape, rate limit, transient failure | No delivered event. Auth/missing-key/validation/rate-limit create no ledger row. Transient `503` commits no new row, so a retry is safe. |

The kind `9` event and delivered ledger row commit atomically and share the
same delivery UUID. A crash after commit and before the HTTP response is safe:
the provider retry returns the existing delivery.

Rate limit: 60 requests / 60 seconds per community + App UUID + transport peer
IP (not `X-Forwarded-For`), after the App has resolved as active. Invalid
secrets count toward the quota. `429` includes `Retry-After`.

## HTTP status and stable codes

| HTTP | `code` | When |
| --- | --- | --- |
| `202` | — | New or identical delivery. Body: `delivery_id`, `event_id`, `status: "delivered"`, `replayed`. |
| `400` | `invalid_app_id` | Path UUID is not a canonical lowercase hyphenated UUID. |
| `400` | `invalid_callback` | Wrong content type, invalid JSON, non-object, unknown/reserved top-level key, or body over 64 KiB. |
| `401` | `unauthorized` | Missing or invalid `X-Webhook-Secret`. Returned **before** content-type, body, JSON, or control-field diagnostics. |
| `404` | `app_not_found` | Unknown community, missing App, cross-community App ID, or disabled App. |
| `409` | `idempotency_conflict` | Key already used with a different canonical payload. |
| `422` | `invalid_control_fields` | Missing/empty/oversized required fields, invalid `event_type`, or invalid `metadata` bounds. No ledger row. |
| `422` | `repository_missing` | No repository matched. |
| `422` | `repository_ambiguous` | More than one repository matched. |
| `422` | `project_missing` | No claim-valid project for the repository. |
| `422` | `project_ambiguous` | More than one claim-valid project. |
| `422` | `project_channel_invalid` | Missing, malformed, archived, deleted, or cross-community destination channel. |
| `422` | `channel_missing` | No live stream channel has the requested `channel_name`. |
| `422` | `channel_ambiguous` | More than one live stream channel has the requested `channel_name`. |
| `429` | `rate_limited` | Quota exceeded. `Retry-After` is seconds until the window resets. |
| `503` | `service_unavailable` | Transient database, Redis, signer, route-read, or mention-read failure. Retry with the same key. |

Redacted messages never include candidate repositories, project or channel
membership, secret hashes, raw bodies, or raw idempotency keys.

The shared workflow router also defines `alias_target_unavailable`,
`route_unauthorized`, and `route_stale`. App callbacks do not emit those:
they use an empty alias map, do not require App channel membership, and do
not revalidate a stored workflow route.

## Rotation and disable

- **Rotate** replaces the stored secret hash immediately. The old secret
  receives `401` / `unauthorized`. Public metadata does not change. Capture the
  new secret from the one-time response and update every provider binding.
- **Disable** keeps kind `39008` queryable so historical messages still render
  as the App. New callbacks receive `404` / `app_not_found`.
- **Enable** restores callbacks with the current secret.
- A lost secret is recovered only by rotating.

## Delivered message and client attribution

A successful callback creates a top-level kind `9` channel message signed by
the community relay. Content is the validated `content` field. Tags:

- `h` — destination channel UUID
- `buzz:app` — App UUID
- `a` — repository coordinate (marker `repository`)
- `a` — project coordinate (marker `project`)
- `buzz:app-delivery` — delivery UUID
- `buzz:app-event` — validated `event_type`

There is no author-attributing `p` tag for the App creator or a workflow
owner. `@Name` mentions in content may add mention `p` tags for current
channel members; those are never the App author. Relays skip workflow
triggers on `buzz:app` messages, so a callback cannot loop through a
workflow.

Clients trust App attribution only when **all** of these hold:

1. The message signature is valid for the **active community relay** key.
2. `buzz:app` is a canonical App UUID.
3. The latest kind `39008` metadata for that UUID is signed by the **same**
   relay in the **same** community.

Any failure — invalid signature, wrong relay key, malformed tags, missing
metadata, or a spoofed `p` tag — renders the actual relay signer. Disabled
metadata still attributes historical messages.

Desktop shows the App name, icon, and an `App` badge in the timeline, thread
roots, quotes, search, supported link previews, and desktop notifications.
Mobile shows the same actor on channel and thread messages.

## Audit and storage

Lifecycle operations write the community audit chain with the admin actor and
App UUID: `app_created`, `app_updated`, `app_secret_rotated`, `app_enabled`,
`app_disabled`. Successful deliveries also write the ordinary `event_created`
audit for the kind `9` event.

The delivery ledger stores hashes, route snapshot, status, event ID, and
redacted failure codes. It does not store the raw secret, raw idempotency key,
request headers, or callback body. Message content lives only on the signed
event.

Logs and metrics use App UUID, delivery UUID, outcome, latency, and
low-cardinality codes.

## Compatibility

Workflow definitions, webhook URLs, workflow secrets, and workflow-owner
attribution are unchanged. During rollout, an old workflow (for example
“Archon Notifications”) may coexist. Disable it only after the App callback
has passed live verification for every bound repository.
