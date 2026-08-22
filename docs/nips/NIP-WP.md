NIP-WP
======

Workspace Profile
-----------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-11 (relay information document), NIP-42 (Authentication of Clients to Relays), NIP-43 (Relay Access Metadata and Requests)

## Abstract

This NIP defines how a relay-scoped workspace icon is set and read. An admin or owner sets it once with a user-signed command (`kind:9033`, accepted only from relay admins/owners); the relay stores it as per-relay state and serves it in the standard `icon` field of its NIP-11 relay information document, where every client — member or not, Buzz or third-party — reads it.

The write path mirrors NIP-43's admin command shape (`kind:9030`–`9032`): user intent is validated against the relay's access-control state, then the relay updates derived state. The read path is plain NIP-11 — no new event kind is needed to consume the icon.

## Motivation

In Buzz the relay *is* the workspace ([VISION.md](../../VISION.md)). A client connected to several relays needs a way to tell them apart that every member sees identically — initials derived from a locally-configured workspace name differ per device and say nothing about the workspace itself.

Upstream Nostr already standardizes the *read* side of this: NIP-11 defines a first-class `icon` field on the relay information document, fetched with an unauthenticated `GET` + `Accept: application/nostr+json`. This NIP adopts that read path unchanged, so any NIP-11-aware client renders the workspace icon with zero Buzz-specific code.

What upstream does not provide is an in-protocol, role-gated **write** path suited to this deployment model:

- **NIP-86 (Relay Management API)** defines a `changerelayicon` method, but it is a separate JSON-RPC/HTTP surface with its own auth model, distinct from the NIP-42/NIP-43 role state Buzz relays already enforce. Buzz's admin surface is Nostr events (kinds 9030–9032); the icon write follows the same shape rather than introducing a second management protocol for one field.
- **NIP-29 group metadata** (`kind:39000` `picture`) is per-group state; the workspace icon is per-relay.

Hence one added command kind (`9033`), validated exactly like the neighboring 9030–9032 membership commands, feeding the standard NIP-11 `icon`.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **actor**: The pubkey that signed a `kind:9033` command.
- **workspace icon**: The image identifying the workspace, carried as an `https` URL or an inline `data:image/*` URL.

## Kinds

| Kind | Name | Signer | Purpose |
|------|------|--------|---------|
| `9033` | Set Workspace Profile | admin / owner | Command: set or clear the workspace icon |

## Event Format

### `kind:9033` Set Workspace Profile

A command signed by a relay admin or owner. The icon value is carried in an `icon` tag; content is empty.

```jsonc
{
  "kind": 9033,
  "pubkey": "<admin-or-owner-pubkey-hex>",
  "content": "",
  "tags": [
    ["icon", "data:image/webp;base64,..."]
  ]
}
```

- exactly one `icon` tag. An empty value (or an absent tag) clears the icon.
- the value MUST be an `https` URL, an `http` URL, or a `data:image/*` URL. Inline data URLs are RECOMMENDED for small icons (≤128px): they render on clients connected to *other* relays without a cross-origin media fetch behind another relay's auth wall.

The `content` field is empty and carries no meaning. Relays MUST NOT parse semantics from `content`.

## Relay Processing Algorithm

When a relay receives a `kind:9033` command it MUST, before applying it:

1. Verify the event signature and NIP-42/NIP-98 authentication as usual.
2. Verify the actor holds the `admin` or `owner` role in the relay's authoritative access-control state (the same state that backs NIP-43). Reject otherwise.
3. Validate the `icon` value: empty (clear), or an `http(s)`/`data:image/*` URL containing no whitespace or control characters, within the relay's size limits. Relays SHOULD cap plain URLs (2048 bytes RECOMMENDED) and inline data URLs (96 KiB RECOMMENDED) and MUST reject non-image `data:` URLs.

On acceptance the relay stores the value as its current workspace icon (per relay — in a multi-tenant deployment, per community) and serves it in the `icon` field of its NIP-11 relay information document. A cleared icon omits the field. Last accepted command wins.

## Client Behavior

1. Fetch the relay's NIP-11 document (`GET` on the relay's HTTP endpoint with `Accept: application/nostr+json`).
2. If the document has a non-empty `icon`, render it wherever the workspace is identified (workspace rail, switcher, settings). Otherwise fall back to a local placeholder (e.g. name initials).

NIP-11 is unauthenticated, so a client can read icons for workspaces it is not currently connected to (e.g. inactive workspaces in a rail) with a plain HTTP fetch. Clients MAY cache the icon locally (keyed by relay URL) to render workspaces whose relays are currently unreachable; the cache is presentation-only and is replaced by the next fetched document.

Only admins/owners can change the icon. Clients SHOULD hide the icon editor from non-admins, but the relay-side role check in §Relay Processing is the enforcement.

## Security Considerations

The icon is intentionally public presentation state: NIP-11 is an unauthenticated document, and serving the icon there means anyone who can reach the relay host can read it. Admins MUST NOT put non-public information in the icon. In a multi-tenant deployment the icon is scoped to the community resolved from the request host — a request can only ever observe the icon of the community it is already addressing, and an unmapped host receives a document with no `icon` field.

Icon values are rendered as images by every member's client, so the relay MUST validate them at the write path: scheme allow-list (`http(s)` / `data:image/*` only — never `javascript:` or non-image `data:` types), no whitespace or control characters, and size caps. Clients render the value in an `<img>`-equivalent sink only, never as HTML.

## Relation to Other NIPs

- **NIP-11 (Relay Information Document)**: Supplies the standard `icon` field and the unauthenticated read path this NIP feeds. Buzz adds nothing to the read side.
- **NIP-43 (Relay Access Metadata and Requests)**: Supplies the role state (`admin` / `owner`) that authorizes `kind:9033`, and the admin-command shape (`9030`–`9032`) it extends.
- **NIP-86 (Relay Management API)**: Standardizes `changerelayicon` over a separate JSON-RPC management surface; this NIP achieves the same mutation in-protocol, gated by the NIP-43 role state the relay already enforces (see §Motivation).
