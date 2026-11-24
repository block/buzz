NIP-WH
======

Webhook Ingress
---------------

`draft` `optional` `relay`

**Depends on** [NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md), [NIP-11](https://github.com/nostr-protocol/nips/blob/master/11.md), [NIP-98](https://github.com/nostr-protocol/nips/blob/master/98.md), and [NIP-AR](NIP-AR.md). The relay applies its [NIP-FI](NIP-FI.md) policy where required.

The key words MUST, MUST NOT, and MAY have the meanings defined in [BCP 14](https://www.rfc-editor.org/info/bcp14) when they appear in capitals.

## Overview

An integration connects an external webhook source to a relay community. A gateway verifies requests from that source and forwards signed deliveries to the relay. Channel administrators can subscribe their channels to a subset of those deliveries. The relay publishes a record in each matching subscription's channel.

| Event | Kind | Signer | Audience |
| --- | --- | --- | --- |
| Integration | To be assigned | Community admin or owner | Community admins and owners, and channel admins and owners in the community |
| Delivery | To be assigned | Authorised gateway | Internal to the relay's delivery processing |
| Subscription revision | `45010` | Destination channel admin or owner | Readers of the destination channel |
| Record | To be assigned | Relay | Readers of the destination channel |

Integration, delivery, and record events are regular stored Nostr events. This draft uses symbolic names until their kind numbers are assigned. Subscription revisions use the NIP-AR envelope. In the formats below, event IDs are 64 lowercase hexadecimal characters and channel and artifact IDs are lowercase, hyphenated, non-nil UUIDs. `content` is a JSON string encoding the object described for that event. Relays MUST reject unsupported versions on ingress and require exactly one occurrence of each specified required tag.

The relay derives the community from the authenticated connection or HTTP origin and resolves all event and channel references within that community.

## Integration

A community admin or owner creates an integration by submitting a signed integration event to the relay's setup endpoint. The relay MUST check the signer's current community role and MUST accept integration writes only through this endpoint. The integration's event ID remains its stable ID. The community owns the integration, and the relay does not depend on the original signer's continuing membership when processing deliveries.

The event has one `["t", "provider:<provider>"]` tag. The provider is a lowercase identifier from the relay's supported provider set. The content has these fields.

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | integer | `1` |
| `name` | nonempty string | Display name |
| `secret_source` | string | `provider` for an externally issued secret or `gateway` for a gateway-issued secret |

### Setup and lifecycle

Use the following HTTPS operations on the community origin. Each requires NIP-98 authentication. A write proof MUST include a payload tag covering the exact request body. For creation, the proof signer MUST match the integration event signer. Only community admins and owners may write. Readers of integration events may list them.

| Request | Body or query | Purpose |
| --- | --- | --- |
| `POST /api/webhook-integrations` | JSON with `event` and optional `secret` | Create and provision the signed integration |
| `POST /api/webhook-integrations/{integration_id}/secret` | JSON with `secret` for a provider-issued secret, or an empty object to issue a gateway secret | Set or rotate a verification secret |
| `POST /api/webhook-integrations/{integration_id}/disable` | Empty JSON object | Disable intake |
| `GET /api/webhook-integrations` | Optional `status=active` | List integrations with their last confirmed status |

The relay MUST keep secrets out of signed events, queries, and lists. For completed creation, it returns `integration_id`, `webhook_url`, and `status`. A gateway-issued secret is returned only when it is issued. The webhook URL stays stable across rotation. The relay may confirm pending provisioning asynchronously. Retrying creation with the same signed event MUST reuse the integration ID and MUST NOT create another integration or rotate its secret.

The list returns a JSON object with an `integrations` array. Each entry has `integration_id`, `name`, `provider`, and a `status` of `pending_secret`, `active`, or `disabled`. Active means the gateway has confirmed it can verify deliveries. Once the gateway applies a disable, it MUST reject new deliveries. Work it already accepted may finish.

## Delivery

The gateway MUST verify the provider's signature over the original request bytes before parsing or transforming the payload. It MUST reject missing or invalid signatures, enforce the provider's freshness rules where available, and verify that the delivery belongs to the configured account or resource. The gateway deduplicates stable provider delivery IDs within each integration when the provider supplies them. It MUST return success to the provider only after it durably stores a verified delivery or recognises an already accepted duplicate.

The gateway signs a delivery event with one integration reference `["e", "<integration-event-id>"]` and one provider tag `["t", "provider:<provider>"]`. Its content has these fields.

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | integer | `1` |
| `type` | nonempty string | Provider event type |
| `delivery_id` | string, optional | Stable provider delivery ID |
| `payload` | JSON object | Verified provider payload |

The gateway sends the event to `POST /events` on the community origin with a fresh, body-bound NIP-98 proof. The relay MUST admit deliveries only through this path. It MUST verify both signatures, require the proof signer to match the event signer, and check that this key has an operator-configured grant to submit deliveries for that community. The integration ID and provider MUST match an integration in that community. On retry the gateway MUST resend the same signed delivery event with a fresh proof. The relay MUST durably accept the event and arrange processing before acknowledging success. A duplicate event ID MUST NOT schedule duplicate processing.

The relay MUST exclude deliveries from client-facing queries, counts, search, and subscriptions. Internal consumers may receive approved fields for their authorised task. A deployment MAY co-locate the gateway and relay if it preserves the same verification and visibility requirements.

## Subscription artifact

A subscription is a NIP-AR artifact of type `buzz.webhook.subscription`. Its `h` is the destination channel and its `d` is the stable subscription ID. This type requires the channel admin or owner role for mutations under [NIP-AR's type-specific write rules](NIP-AR.md#type-specific-write-permissions). Use normal channel read permission.

On creation, update, and restore, the artifact MUST have one `["e", "<integration-event-id>"]` tag. The integration and destination channel are fixed for the life of the subscription. The relay MUST reject `move` or a thread anchor on this type. The relay MUST require an active integration in the same community when creating or restoring a subscription. A delete uses the normal NIP-AR tombstone, which omits the integration tag and content. The relay preserves the original integration binding across deletion so it can validate a restore.

| Content field | Type | Meaning |
| --- | --- | --- |
| `version` | integer | `1` |
| `filter` | nonempty string | [CEL](https://cel.dev/) expression returning a boolean |
| `transform` | string, optional | CEL expression returning a JSON object |

The evaluation environment exposes `event.provider`, `event.type`, and `payload`. The last contains only the fields the provider adapter allows for that event type. The relay MUST validate the expressions before accepting the revision and apply execution limits during evaluation. Only a true filter and a successful transform, if present, produce a match. Evaluation failures MUST NOT publish a partial record or prevent other subscriptions from being evaluated.

The relay MUST use only the current accepted, live artifact revision for a new evaluation and keep the matching index consistent with NIP-AR acceptance, deletion, redaction, and channel lifecycle. A record produced by work already in progress identifies the exact accepted revision it evaluated.

Clients list current subscriptions using NIP-AR's `artifact: "current"` view with `#h` set to the destination channel and `#type` set to `buzz.webhook.subscription`.

## Record

For each match, the relay signs one record scoped to the subscription's channel. It includes these tags.

| Tag | Value |
| --- | --- |
| `h` | Destination channel UUID |
| `e` | Integration event ID |
| `s` | Stable subscription artifact ID |
| `t` | `provider:<provider>` |

| Content field | Type | Meaning |
| --- | --- | --- |
| `version` | integer | `1` |
| `type` | nonempty string | Provider event type |
| `delivery_event_id` | event ID | Source delivery ID |
| `subscription_revision` | event ID | Exact accepted subscription revision evaluated |
| `payload` | JSON object, optional | Result of the subscription's transform, when defined |

The relay MUST publish at most one record per community, source delivery event ID, and stable subscription ID, including across retries and subscription edits. It MUST enforce the destination channel's read permission on every record read. Submission of records is reserved for the relay. Raw delivery payloads MUST NOT be copied into records, and the delivery ID MUST NOT grant access to their contents.

Clients can query or subscribe to records using the record kind and `#h`. Add `#e` to filter by integration or `#s` to filter by subscription across revisions. A record is input to automation. For example, a channel workflow could read a pull request title from a transformed record and render a normal kind `9` message in the same channel.

## Relay discovery

A relay that implements this protocol advertises `"webhook_ingress": {"version": 1}` in its NIP-11 document. Clients MUST check support before using setup or submitting subscription artifacts. The relay MUST advertise support only when every backend serving the community enforces the NIP-WH admission, access, and subscription validation rules.
