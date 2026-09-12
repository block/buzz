NIP-WP
======

Workspace Profile
-----------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-11 (relay information document), NIP-42 (Authentication of Clients to Relays), NIP-43 (Relay Access Metadata and Requests)

## Abstract

This NIP defines how relay-scoped workspace profile fields are set and read. An admin or owner sets profile fields with a user-signed command (`kind:9033`); the relay stores them as per-relay state and serves them in its NIP-11 relay information document. The standard `icon` field lets every client — member or not, Buzz or third-party — read the workspace icon. Buzz-specific fields, such as `thread_replies_in_channel`, advertise workspace behavior to Buzz-aware clients.

The write path mirrors NIP-43's admin command shape (`kind:9030`–`9032`): user intent is validated against the relay's access-control state, then the relay updates derived state. The read path is plain NIP-11 — no new event kind is needed to consume workspace profile fields.

## Motivation

In Buzz the relay *is* the workspace ([VISION.md](../../VISION.md)). A client connected to several relays needs a way to tell them apart that every member sees identically — initials derived from a locally-configured workspace name differ per device and say nothing about the workspace itself.

Upstream Nostr already standardizes the *read* side of this: NIP-11 defines a first-class `icon` field on the relay information document, fetched with an unauthenticated `GET` + `Accept: application/nostr+json`. This NIP adopts that read path unchanged, so any NIP-11-aware client renders the workspace icon with zero Buzz-specific code.

What upstream does not provide is an in-protocol, role-gated **write** path suited to this deployment model:

- **NIP-86 (Relay Management API)** defines a `changerelayicon` method, but it is a separate JSON-RPC/HTTP surface with its own auth model, distinct from the NIP-42/NIP-43 role state Buzz relays already enforce. Buzz's admin surface is Nostr events (kinds 9030–9032); the icon write follows the same shape rather than introducing a second management protocol for one field.
- **NIP-29 group metadata** (`kind:39000` `picture`) is per-group state; the workspace icon is per-relay.

Hence one added command kind (`9033`), validated like the neighboring 9030–9032 membership commands, feeding the standard NIP-11 `icon` and Buzz-specific NIP-11 extension fields.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **actor**: The pubkey that signed a `kind:9033` command.
- **workspace icon**: The image identifying the workspace, carried as an `https` URL or an inline `data:image/*` URL.
- **thread replies in channel**: A community setting that tells Buzz clients to show thread replies as flat chronological channel-window rows. It is a read/display projection, not a duplicate send operation.

## Kinds

| Kind | Name | Signer | Purpose |
|------|------|--------|---------|
| `9033` | Set Workspace Profile | admin / owner | Command: set or clear workspace profile fields |

## Event Format

### `kind:9033` Set Workspace Profile

A command signed by a relay admin or owner. Profile fields are carried in tags; content is empty.

```jsonc
{
  "kind": 9033,
  "pubkey": "<admin-or-owner-pubkey-hex>",
  "content": "",
  "tags": [
    ["icon", "data:image/webp;base64,..."],
    ["thread_replies_in_channel", "true"]
  ]
}
```

- `icon`: optional. An empty value clears the icon. If no recognized profile tag is present, a legacy empty command clears the icon.
- `thread_replies_in_channel`: optional boolean. Accepted true values are `true`, `1`, `yes`, and `on`; accepted false values are `false`, `0`, `no`, and `off`. Unknown values MUST be rejected. When true, NIP-CW channel windows for the community include direct and nested thread replies as flat chronological row projections.
- the `icon` value MUST be an `https` URL, an `http` URL, or a `data:image/*` URL. Inline data URLs are RECOMMENDED for small icons (≤128px): they render on clients connected to *other* relays without a cross-origin media fetch behind another relay's auth wall.

The `content` field is empty and carries no meaning. Relays MUST NOT parse semantics from `content`.

## Relay Processing Algorithm

When a relay receives a `kind:9033` command it MUST, before applying it:

1. Verify the event signature and NIP-42/NIP-98 authentication as usual.
2. Verify the actor holds the `admin` or `owner` role in the relay's authoritative access-control state (the same state that backs NIP-43). Reject otherwise. A relay MAY admit an icon-only `9033` from any authenticated sender on an open relay that has no admin/owner row yet, but that bootstrap exception MUST NOT authorize `thread_replies_in_channel`.
3. Validate each supplied profile field. The `icon` value is empty (clear), or an `http(s)`/`data:image/*` URL containing no whitespace or control characters, within the relay's size limits. Relays SHOULD cap plain URLs (2048 bytes RECOMMENDED) and inline data URLs (96 KiB RECOMMENDED) and MUST reject non-image `data:` URLs. The `thread_replies_in_channel` value MUST parse as a boolean.

On acceptance the relay stores supplied values as its current workspace profile (per relay — in a multi-tenant deployment, per community) and serves them in its NIP-11 relay information document. A cleared icon omits the field. Last accepted command wins per field.

## Client Behavior

1. Fetch the relay's NIP-11 document (`GET` on the relay's HTTP endpoint with `Accept: application/nostr+json`).
2. If the document has a non-empty `icon`, render it wherever the workspace is identified (workspace rail, switcher, settings). Otherwise fall back to a local placeholder (e.g. name initials).
3. If the document has `thread_replies_in_channel: true`, render NIP-CW channel-window replies as channel rows according to NIP-CW's projection rules. Missing/false means the default channel-window row mode.

NIP-11 is unauthenticated, so a client can read icons for workspaces it is not currently connected to (e.g. inactive workspaces in a rail) with a plain HTTP fetch. Clients MAY cache the icon locally (keyed by relay URL) to render workspaces whose relays are currently unreachable; the cache is presentation-only and is replaced by the next fetched document.

Only admins/owners can change `thread_replies_in_channel`. Clients SHOULD hide profile editors from non-admins, but the relay-side role check in §Relay Processing is the enforcement. Implementations that support the rosterless-open-relay icon bootstrap exception SHOULD still hide it once a steward exists.

## Security Considerations

Workspace profile fields are intentionally public presentation/configuration state: NIP-11 is an unauthenticated document, and serving the profile there means anyone who can reach the relay host can read it. Admins MUST NOT put non-public information in the icon. In a multi-tenant deployment the profile is scoped to the community resolved from the request host — a request can only ever observe the profile of the community it is already addressing, and an unmapped host receives a document with default/empty profile fields.

Icon values are rendered as images by every member's client, so the relay MUST validate them at the write path: scheme allow-list (`http(s)` / `data:image/*` only — never `javascript:` or non-image `data:` types), no whitespace or control characters, and size caps. Clients render the value in an `<img>`-equivalent sink only, never as HTML.

`thread_replies_in_channel` changes only read presentation and channel-window row selection. It MUST NOT publish duplicate channel events or expose replies outside the community/channel access scope enforced by NIP-CW.

## Relation to Other NIPs

- **NIP-11 (Relay Information Document)**: Supplies the standard `icon` field and the unauthenticated read path this NIP feeds. Buzz adds `thread_replies_in_channel` as an extension field for Buzz-aware clients.
- **NIP-43 (Relay Access Metadata and Requests)**: Supplies the role state (`admin` / `owner`) that authorizes `kind:9033`, and the admin-command shape (`9030`–`9032`) it extends.
- **NIP-CW (Channel Window)**: Consumes `thread_replies_in_channel` as the community row mode that determines whether thread replies are projected into channel-window rows.
- **NIP-86 (Relay Management API)**: Standardizes `changerelayicon` over a separate JSON-RPC management surface; this NIP achieves the same mutation in-protocol, gated by the NIP-43 role state the relay already enforces (see §Motivation).
