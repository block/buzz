NIP-MG
======

Community Mention Groups
------------------------

`draft` `optional` `relay` `Buzz extension`

**Depends on:** NIP-01 (events and filters), NIP-11 (discovery), NIP-42
(authentication), NIP-70 (protected relay state).

## Purpose and scope

A mention group is a community-owned name and explicit set of recipient public
keys. People and agents share the same representation. The relay executes signed
commands and publishes authoritative state; the client does not own or sign the
authoritative directory. These numbers are Buzz extensions, not global Nostr
allocations.

A group has no channel, conversation roster, or creator-owned authority. Its
optional default channels identify existing public channels to join when members
are added. Inclusion never grants private-channel access. Saving group state
must not itself create a message, mention notification, unread item, or agent
conversation wake.

Every command, receipt, group and state event belongs to the server-resolved
community. Clients must bind editors and pending results to that community
session; identical UUIDs on two communities refer to independent groups.

## Kinds and discovery

| Kind    | Name                  | Author           | Meaning                                              |
| ------- | --------------------- | ---------------- | ---------------------------------------------------- |
| `47000` | Put mention group     | Authorized actor | Complete group replacement with version precondition |
| `47001` | Delete mention group  | Authorized actor | Permanent retirement with version precondition       |
| `47002` | Set management policy | Authorized actor | Replace the community management setting             |
| `39010` | Mention-group state   | Relay only       | Addressable group, tombstone, or settings head       |

A supporting relay advertises `nip-mg` in NIP-11 `supported_extensions` and:

```json
{
  "mention_groups": {
    "version": 1,
    "put_kind": 47000,
    "delete_kind": 47001,
    "set_manage_kind": 47002,
    "state_kind": 39010,
    "default_manage": "everyone",
    "membership": "roster",
    "max_members": 20
  }
}
```

`version` is the protocol schema version, independent of state revisions.
`membership` is `open` or `roster`: on an open community, any authenticated,
unbanned key is a current member; on a roster community, current membership
requires a community member row or an effectively admitted agent through its
owner. The advertised message-size limit is
`limitation.max_message_length`. A host must advertise its command capability
only when its signing and publishing paths support this contract.

## Authority and admission

Management defaults to `everyone`. Under `everyone`, current community members
may put and delete groups. Under `admins`, the signer must hold an actual
community `owner` or `admin` role; an agent does not inherit that role from its
owner. Group inclusion grants no management authority.

Changing the management policy requires an actual owner/admin role. If the
community has no owner/admin row, any current member may change the setting.
This exception permits stewardship on communities without an initial steward.
It does not grant inherited admin authority to agents in a community that has a
steward. On an open community without a steward, any authenticated, unbanned key
may therefore change the management policy.

Commands use authenticated WS `EVENT` or HTTP `/events`. The signer must equal
the authenticated actor. Existing tenant write fences, credential scope and
ban checks apply. New execution uses the relay-admin freshness window of
plus/minus 120 seconds. The relay evaluates current membership, management policy
and steward authority inside the admitted transaction, ordered against relevant
membership and policy changes.

## Command format

All commands have empty `content`. Singleton semantic fields must appear exactly
once with exactly two tag elements. Unknown tags and inappropriate fields are
rejected, including `h`, state-only fields, and `d` on a policy command.

Put (`47000`) is a complete replacement:

```json
{
  "kind": 47000,
  "content": "",
  "tags": [
    ["d", "0659678f-fddb-40df-a285-35f6b41db019"],
    ["prev", "0"],
    ["name", "Support team"],
    ["p", "<person public key>"],
    ["p", "<agent public key>"],
    ["channel", "865a455d-914c-4b8b-8295-0b6d75f45730"]
  ]
}
```

Delete (`47001`) has exactly `d` and `prev` semantic tags; it has no `name`,
`p`, or `channel` tags. Set policy (`47002`) has exactly `manage` and `prev`
semantic tags, for example `[["manage","admins"],["prev","0"]]`.
`manage` accepts exactly `admins` or `everyone`.

Validation and normalization:

- `d` is a canonical lowercase, hyphenated, non-nil UUID. The literal `settings`
  is reserved for relay state and cannot identify a group.
- `prev` is a canonical decimal string in `0..9223372036854775807`, with no
  whitespace, sign or leading zeroes except the value `0` itself.
- Names are trimmed and contain 1–120 Unicode scalar values. Control characters
  and embedded Unicode line/paragraph separators are rejected. Spelling, case
  and internal spaces are preserved. Active names are unique per community
  under the database's `lower(name)` collation, not full Unicode case folding.
- Each `p` has exactly two elements and a 64-character hexadecimal public key.
  Normalize keys to lowercase, sorted and deduplicated. Zero members is valid;
  at most 20 distinct members are allowed, advertised as `max_members` in NIP-11.
  The signer is included only when explicitly tagged. Builders that suppress
  signer `p` tags must disable that behavior for this protocol.
- Each `channel` has exactly two elements and a canonical non-nil UUID. Sort and
  deduplicate them; at most 32 distinct defaults are allowed. The relay validates
  newly selected channels' eligibility within the community. A saved default
  that later becomes private or unlisted remains an inert reference and does
  not prevent other group edits. Core wire validation performs no channel or
  membership lookup.
- The member limit matches the mobile push recipient gate. Larger audiences
  should use a channel. A put over the limit is refused with
  `invalid: a group can have at most 20 members`. A message combining groups or
  other mentions remains subject to the push recipient gate for the whole
  message. Also reject a command when its resulting signed state exceeds the
  advertised message-size bound; never truncate members or channels.

The only optional transport annotations are one each of:

- `client-id`: two elements, nonempty text without controls, at most 128 UTF-16
  code units, matching the existing host bound;
- `ms`: two elements with a canonical decimal string in `0..999`;
- `auth`: the four-element NIP-OA envelope, with a 64-character hex owner key,
  conditions bounded to 4,096 UTF-8 bytes and 128-character hex signature.
  Existing relay NIP-OA verification remains authoritative; the wire parser does
  not treat a structurally valid annotation as authorization.

Duplicate annotations are rejected. Annotations cannot change command meaning.

## Revisions, transactions and retries

Each group starts unused. A fresh put with `prev=0` creates live revision 1.
Thereafter, a put or delete must supply the current revision and advances it by
one. Counters never wrap. Even an otherwise identical fresh put advances the
revision. A delete commits a permanent tombstone and releases the live name;
the UUID cannot be reused. An empty live group continues to reserve its name.

The settings coordinate has an implicit initial revision 0 and value `everyone`.
There is no signed initial head. Reads must not lazily write a default head. The
first accepted policy command uses `prev=0` and publishes settings revision 1.
Subsequent policy changes use the same compare-and-set revision rules.

Accepted signed command IDs have durable receipts shared by all three command
kinds. An exact retry acknowledges its original acceptance without applying a
second mutation, advancing any revision, repeating default-channel joins, or
publishing another state event. This holds even after a later edit supersedes
the accepted result. A receipt confirms historical acceptance, not currentness.

Receipt replay still requires signature, authenticated actor, tenant, scope,
write-fence and ban admission. It precedes freshness and new-execution authority
checks. A never-accepted stale command remains rejected. On an uncertain outcome,
clients retain and resend the same signed event. After a version conflict they
read current state and reconcile rather than automatically overwriting it.

Group/settings mutation, version check, receipt, signed state insertion and
previous-head retirement commit atomically in one admitted transaction.
Coordinate locking includes absent coordinates so concurrent creates cannot
both succeed. Name uniqueness is enforced transactionally across different IDs.
Failures leave no partial group, setting, receipt, head, or joins.

## Default-channel behavior

On creation, members are new additions. On an update, only keys newly added to
the stored member set are candidates for joining default channels. Replacing
only the default-channel list does not retroactively join existing members.
The relay uses the accepted replacement's defaults and existing channel-join
policy; it skips identities that have left the community and channels that are
gone or closed, including saved defaults that became private or unlisted (counted
as `channel_closed`). Private-channel access is never acquired through a group.
Group deletion does not remove channel membership.

Default-channel membership writes commit in the group transaction. When any
target/channel pair is skipped, the success message is `joins:` and a space followed by
JSON, for example `joins: {"joined":2,"skipped":{"channel_closed":1}}`.
`joined` counts new or reactivated memberships; `skipped` maps stable reasons
to pair counts. Reasons are `channel_unavailable`, `channel_closed`,
`target_not_admitted`, `already_member`, `target_owner_missing`,
`target_owner_only`, `target_policy_nobody`, and `membership_denied`.
An exact retry returns the stored report after `duplicate:` and a space, for example
`duplicate: joins: {"joined":2,"skipped":{"channel_closed":1}}`.
Without skips, first acceptance has an empty message and replay has `duplicate:`.

## Relay state

The relay publishes kind `39010` under its advertised community signing key.
Each current head has empty content, a protected `["-"]` marker, one `d`, one
positive decimal `version`, and one `command` carrying the accepted command ID.

- A live group head uses `d=<group UUID>`, its normalized `name`, sorted `p`
  members, and sorted `channel` defaults.
- A tombstone uses `d=<group UUID>` and `["deleted","true"]`; it has no name,
  member, or default-channel tags.
- A settings head uses `d=settings` and one `manage` tag; it has no name,
  member, channel or deleted tags.

Heads are stored without a channel ID. A tombstone is a discoverable current
event, not a deleted event-store row, and does not expire under ordinary message
retention. Kind-5 deletion cannot remove these relay-owned heads. Users cannot
remove state by deleting their input command. Clients
must reject submitted projections: `39010` is relay-only.

The new head's timestamp is at least `max(now, prior coordinate timestamps + 1)`,
including retired heads, so addressable-event replacement works within one
second. Clients additionally fold by revision, ignore lower revisions and
duplicate event IDs, and repair conflicting equal revisions by reading the
authoritative head.

All admitted community readers can read every head, including nonrecipients and
members without channel memberships. State `p` tags are data, never read gates
or notification intent.

## Reads and convergence

Use ordinary WS `REQ` for history plus live updates:

```json
{"kinds":[39010],"authors":["<relay public key>"],"limit":1000}
{"kinds":[39010],"authors":["<relay public key>"],"#d":["settings"],"limit":1}
```

The directory includes the reserved settings coordinate; distinguish it from
UUID group coordinates. HTTP `/query` pages heads with the existing composite
`until` plus `before_id` cursor, using the last event's inclusive timestamp and
ID. Do not decrement the second. Request strong consistency for edit readback.
Continue until a short successful page; a partial result does not prove absence.
COUNT uses the same tenant/read admission.

Keep the live subscription open while loading pages and merge by coordinate and
revision. Reconcile all heads, including tombstones and settings, at startup and
after interrupted continuity. This is a convergent directory, not a multi-page
atomic snapshot. Share the host's reconnect owner and fleet backoff.

Committed state is durable; normal operation fans out the committed event after
commit. Delivery through publisher failure or process death is not guaranteed by
this protocol's asynchronous dispatch. Reconnect reconciliation and the host's
existing online state-refresh policy repair missed updates. A publish failure
after commit cannot cause a second logical write.

## Responses

WS `OK` and HTTP `/events` acknowledge the signed command event ID.

| Outcome                                    | Accepted | Message prefix              | HTTP |
| ------------------------------------------ | -------- | --------------------------- | ---- |
| Committed                                  | true     | empty                       | 200  |
| Exact accepted retry                       | true     | `duplicate:`                | 200  |
| Wrong revision, retired ID, unknown delete | false    | `conflict:`                 | 409  |
| Another live group owns name               | false    | `duplicate:`                | 409  |
| Insufficient authority                     | false    | `restricted:`               | 403  |
| Banned actor                               | false    | existing `blocked:` refusal | 403  |
| Invalid shape or excessive resulting state | false    | `invalid:`                  | 400  |
| Persistence/admission lookup failure       | false    | `error:`                    | 500  |

Clients must preserve name collision, version conflict, authority denial and
unknown transport outcome as distinct results.

## Community removal and client projection

Community removal leaves stored group keys and revisions unchanged. The host
intersects the stored set with authoritative effective community membership for
completion, sending and reader chips, including correctly owner-admitted agents.
The editor hides confirmed removed identities; its next successful replacement
prunes them. Loading, failed or incomplete discovery is not removal evidence.
Current projections do not rewrite recipients captured in historical messages.

Group expansion still obeys destination admission and send-recipient limits.
The host owns completion aggregation, selected-occurrence provenance, reader
chips and signing/publishing capabilities. Relay support alone does not provide
those client behaviors.

## Deployment

Tenant tables and purge catalogs require the repository's compatibility-first
two-release rollout. All serving deletion executors and operator tools must
recognize the future tables before feature schema is installed. Signed heads,
receipts and group identities remain tenant-scoped through deletion and purge.
See [database guidance](../database.md).
