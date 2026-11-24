NIP-WH
======

Webhook Subscriptions
---------------------

`draft` `optional` `relay`

This NIP defines how channel admins choose which webhook deliveries reach their channel. A community has webhook integrations, each fed by one external provider such as a source host or an incident tool. A channel admin subscribes the channel to an integration. A subscription holds a filter, a [CEL](https://cel.dev) expression that decides whether a delivery matches, and an optional transform, an expression that shapes what the channel receives.

A subscription is a [NIP-AR](NIP-AR.md) artifact of type `buzz.webhook-subscription`. NIP-AR gives it its identity, revisions, conflicts, moves, deletes and reads. This NIP adds the type's tags and content, and one write rule: only channel owners and admins write subscriptions.

This document covers configuring subscriptions. Deliveries and the records published for them are added to it when a relay accepts them.

## Event format

A subscription revision is a NIP-AR `kind:45010` event (standard signing fields omitted):

```jsonc
{
  "kind": 45010,
  "tags": [
    ["ar", "1"],
    ["d", "<subscription UUID>"],
    ["h", "<channel id>"],
    ["type", "buzz.webhook-subscription"],
    ["title", "Opened pull requests"],
    ["op", "create"],
    ["integration", "<integration id>"]
  ],
  "content": "<JSON SubscriptionContent>"
}
```

Every revision except a delete MUST have exactly one `integration` tag with exactly one value: the integration ID, a lowercase hyphenated UUID, as the ingress gateway assigns it. An update MAY change it. A delete follows NIP-AR: empty content and envelope tags only.

```typescript
type SubscriptionContent = {
  version: 1
  filter: string     // CEL expression that returns a boolean
  transform?: string // CEL expression that returns a map
}
```

Unknown fields are an error. Every revision carries the whole filter and transform, as every NIP-AR revision carries a complete snapshot.

### Expressions

Both expressions read two variables:

| Variable | Value |
|----------|-------|
| `event.provider` | The integration's provider |
| `event.type` | The provider's event type, e.g. `pull_request` |
| `payload` | The provider's JSON body |

```
event.type == "pull_request" && has(payload.action) && payload.action == "opened"
```

The CEL standard library is available, except the `timestamp` and `duration` functions. Each expression is at most 4096 bytes and 64 levels deep, and names no variable but these two. CEL is dynamically typed here: the relay refuses a result that is a literal of the wrong type when the revision arrives, and treats any other wrong result as an evaluation error.

## Relay behavior

A relay that supports this NIP lists `buzz.webhook-subscription` in its NIP-11 `artifacts.admin_types` (see NIP-AR Access). It then applies, in addition to every NIP-AR rule:

1. **Shape.** Except on a delete, the `integration` tag and content are well formed and both expressions compile, as described under [Expressions](#expressions).
2. **Admins only.** The signer is an owner or admin of the home channel, and for a move of the source channel too. The relay checks the role in the transaction that accepts the revision, so a demotion that commits first refuses the write.

The relay does not check that the integration exists or is active. A subscription that names an integration this community does not have never matches a delivery.

| Outcome | `OK` | HTTP |
|---------|------|------|
| Accepted | `true`, empty message | 200 |
| Same event sent again | `true`, `duplicate:` | 200 |
| Malformed `integration` tag or content | `false`, `invalid: …` | 400 |
| Expression does not compile | `false`, `invalid: filter: …` or `invalid: transform: …` | 400 |
| Signer is not an owner or admin | `false`, `restricted: only channel admins can write buzz.webhook-subscription artifacts` | 400 |
| Other NIP-AR outcomes (conflict, unknown `prev`, channel gates) | as NIP-AR | as NIP-AR |

### Evaluating

A subscription is evaluated only while the signer of its current revision is an owner or admin of its home channel. A revision signed before the relay enforced this NIP, or by an admin who has since left or lost the role, matches no delivery until a current admin saves a new revision. This protects channels from subscriptions written before the rule existed, without migrating them.

## Reading

Subscriptions are read through NIP-AR. To list a channel's subscriptions and follow changes:

```json
["REQ", "s1", {"artifact": "current", "#h": ["<channel id>"], "#type": ["buzz.webhook-subscription"]}]
```

Anyone who can read the channel can read its subscriptions. Put no secrets in an expression or the title.

## Limits summary

| Item | Limit |
|------|-------|
| `integration` tags on a revision | exactly 1, except on delete |
| Expression size | 4096 bytes |
| Expression depth | 64 levels |
