NIP-CS
======

Conversation Subjects
---------------------

`draft` `optional` `relay`

**Depends on:** NIP-01 (signed events and filters), NIP-29 (channels).

## Abstract

A **subject** is an identifiable object with a canonical conversation location.
This NIP defines tags for that relationship, how several subjects may occupy the
same location, and how declarations are validated. It allocates no event kind.
Each participating kind specifies its content, permitted relationship mode, and
additional authorization requirements.

MUST, MUST NOT, SHOULD, and MAY are interpreted as in RFC 2119.

## Declaration tags

A declaration is a complete state revision, not a patch. All defined tags have
exactly two string elements. Required tags occur exactly once; optional tags at
most once. Duplicate tags are invalid even when their values agree.

| Tag | Cardinality | Value |
|-----|-------------|-------|
| `cs` | 1 | Protocol version: `1` |
| `d` | 1 | Subject UUID, lowercase canonical hyphenated form, non-nil |
| `h` | 1 | Authorization channel UUID, same encoding as `d` |
| `location` | 1 | `channel` or `thread` |
| `root` | 0 or 1 | Thread anchor event ID: 64 lowercase hex characters |
| `claim` | 1 | `exclusive`, `primary`, or `attachment` |
| `prev` | 0 or 1 | Previous accepted revision's event ID |

`root` MUST be absent for a channel location and present for a thread location.
The location key is `(community, h)` for a channel, or `(community, h, root)` for
a thread. A channel claim does not reserve every thread within that channel.

`d` identifies the subject within its community, independent of author and kind.
It is not a NIP-01 addressable coordinate. Participating kinds use regular stored
events; the relay retains revisions and maintains their current heads separately.

Unknown tags MAY carry non-authoritative annotations and MUST NOT change these
semantics. Unsupported `cs` versions MUST be rejected. Type specifications may
reserve additional tags and restrict their cardinality. Relationships MUST NOT
also be encoded as independently authoritative copies in `content`.

## Claim modes

| Existing claim | New exclusive | New primary | New attachment |
|----------------|---------------|-------------|----------------|
| None | Allow | Allow | Allow |
| Exclusive | Reject | Reject | Reject |
| Primary | Reject | Reject | Allow |
| Attachment(s) only | Reject | Allow | Allow |

An **exclusive** subject is the only subject at its location. A **primary**
subject is the only primary subject, but permits attachments. An **attachment**
shares a location without reserving its primary position. Attachment does not
require a primary subject to exist.

The concrete kind fixes its mode; a writer MUST NOT select a weaker mode to
bypass a constraint. Revisions of the same subject do not compete with its own
claim. Claims are checked across all participating kinds, not per kind.

## Identity and revisions

Creation omits `prev`. It reserves an unused `d`; its signer becomes the subject
controller. An update includes `prev` naming the current accepted head, uses the
same kind and `d`, and is signed by that controller. Channel administration alone
does not confer control of another subject.

The relay MUST compare the head, validate relationships, store the event, and
advance the head in one transaction. Competing updates to the same head cannot
both succeed. A repeated event ID is an idempotent duplicate, not a new revision.
`created_at` does not determine the winning head. Rejected events are not
broadcast or included in queries.

The channel, location, root, claim mode, and controller are immutable. Type
specifications identify additional immutable fields. Retirement is a
kind-specific state transition, not a transfer of the conversation. Deletion
requests MUST NOT release claims or remove a subject's anchor while references
remain; clients use retirement instead of NIP-09 deletion for subject state.

## Thread anchors

`root` names an existing, live, channel-scoped conversation anchor in `h`.
It MUST resolve within the same community. An anchor's event ID remains stable
when any subject metadata changes. Subject declarations are not conversation
replies: `e` root/reply markers MUST NOT be used to express the home relationship.
They MUST NOT increment reply counters or become ordinary timeline messages.

Replies use the channel's existing thread protocol and reference the anchor,
not a subject's latest revision. For nested conversations, the home anchor can
be a descendant; its outer root and immediate-parent ancestry remain intact.
A subject declaration does not rewrite that ancestry or introduce membership.

## Authorization

The relay MUST check access to `h` on creation and every update. Exclusive and
primary claims additionally require channel administration; attachments require
channel write permission. Concrete kinds may require additional source-object
authority. Being able to read an object is not permission to disclose it to a
different channel.

Every subject revision is channel-scoped by `h`. Queries, direct-ID reads,
counts, search, and live delivery MUST apply that channel's visibility rules.
The existence of an unreadable subject MUST NOT be disclosed through a failed
relationship lookup. A link grants neither source-object capabilities nor
conversation access. Thread access always derives from its one containing
channel, never the union or intersection of linked objects' audiences.

## Retrieval

Writes use Nostr `EVENT`; the generic HTTP equivalent is `POST /events`.
History and live updates use `REQ`; `POST /query` provides the corresponding
bounded historical query. No type-specific CRUD endpoint is required.

For a participating kind `K`:

```jsonc
// Subject revisions, with live updates after EOSE.
["REQ", "subject", {"kinds": ["<K>"], "#h": ["<channel>"], "#d": ["<subject>"]}]
```

`<K>` denotes a numeric kind, not a literal string on the wire. Filters on `d`
MUST be applied before LIMIT even though these kinds are not addressable events.
History is revision history, not a current-state list. Clients fold accepted
revisions using `prev`, deduplicating by event ID; an incomplete chain requires
fetching missing revisions rather than selecting the largest timestamp.

Location queries use `#h` and `#root` semantics through the relay's generic query
extension; multi-character tag filtering is a Buzz extension, not guaranteed by
NIP-01. All relationship predicates and authorization MUST run before pagination.
A relay MUST reject unsupported relationship filters rather than ignore them.
EOSE means the historical response ended; it is not a durable replay cursor.

## Invalid declarations

- Two `h` tags, even if identical.
- A thread location without `root`, or a channel location with `root`.
- An exclusive claim at a location already occupied by an attachment.
- A second primary subject at a location with an existing primary.
- An update with a stale `prev`, different signer, or changed home.
- A thread anchor in a different channel or community.
