NIP-IM
======

Import Provenance
-----------------

`draft` `optional` `client`

**Depends on**: NIP-01 (basic event format)

## Abstract

This NIP defines a tag convention marking an event as imported from an external
system, and identifying which record in that system it came from. It adds no
event kind, no relay behaviour, and no schema. It standardises one spelling so
that independently written importers and bridges agree, and so clients can
render imported content distinctly without special-casing each tool.

```json
["imported_from", "<source>", "<source_ref>"]
```

## Motivation

Buzz communities are increasingly populated from elsewhere: history imported
from Slack, Discord or Mattermost archives, and messages forwarded by live
bridges such as the Slack Connect bridge discussed in block/buzz#2822.

These tools each need to answer the same three questions, and today each would
answer them differently:

1. **Is this event imported?** Clients want to render archived history with an
   affordance distinguishing it from live conversation — a badge, muted styling,
   a "this was imported" note. Without a convention, every client needs to know
   every importer.
2. **Which external record produced it?** Importers must be idempotent. Imports
   fail halfway and get resumed, and a resumed import must not duplicate what it
   already published. That requires asking the relay "did I already import
   record X?"
3. **Which import produced it?** Operators need to attribute, audit and
   potentially tombstone a specific import without touching organically authored
   events.

Nothing in Nostr answers these. NIP-01's `e` and `p` tags reference events and
pubkeys inside the network. There is no primitive for "this content originated
outside it".

The convention is worth standardising precisely because it requires no code. It
is cheap enough that every importer will invent *something*; the only question
is whether they invent the same thing.

## Non-Goals

**This NIP does not assert authorship.** An `imported_from` tag says where
content came from, not who wrote it. Importers that publish under derived or
custodial keys are making a separate claim, and this tag neither supports nor
launders it. A client MUST NOT treat the presence of this tag as evidence that
the signing key belongs to the original author.

**This NIP does not define trust.** Any pubkey can add this tag to any event.
It is a self-asserted provenance hint, not a proof. Clients MUST NOT grant
imported events any authority they would not grant the signing key alone.

**This NIP does not define import mechanics.** How events are produced,
authenticated, rate-limited, or timestamped is out of scope. In particular it
says nothing about historical `created_at`, which Buzz constrains separately
(see `migrations/0021_created_at_fence_floor.sql`).

**This NIP does not define deletion or reconciliation.** Removing or updating a
previous import uses existing primitives (NIP-09).

**This NIP does not require relay support.** Relays that know nothing about it
behave correctly, because they already store and index arbitrary tags.

## Specification

### Tag format

An event MAY carry at most one `imported_from` tag:

```
["imported_from", <source>, <source_ref>]
```

- `<source>` — REQUIRED. A short, lowercase identifier for the originating
  system. It SHOULD be the system's common name with no version or vendor
  qualifier: `slack`, `discord`, `mattermost`, `teams`, `irc`. Clients matching
  on this value MUST treat it case-sensitively; producers MUST emit lowercase.
- `<source_ref>` — REQUIRED. An opaque, stable identifier for the specific
  record in the source system. It MUST be stable across re-runs of the same
  import: this is what makes idempotency possible.

`<source_ref>` is opaque to relays and to clients that do not recognise
`<source>`. Its internal structure is defined per source, and producers SHOULD
use a slash-delimited path from the source's own identifiers, most general
first. For Slack:

```
["imported_from", "slack", "T024BE7LH/C0GENERAL/1709545400.000400"]
```

that is `<team_id>/<channel_id>/<message_ts>`, using Slack's own ids verbatim.
Reformatting them — normalising a timestamp, lowercasing an id — breaks
idempotency and MUST NOT be done.

Producers SHOULD include every component needed to make the reference globally
unique, because a workspace id alone does not distinguish two exports of two
different workspaces.

### Producer requirements

A producer MUST emit the same `<source_ref>` for the same source record on every
run. A producer SHOULD apply the tag to every event it derives from an external
record, including reactions and profile events, not only to messages — a
partially tagged import cannot be audited or resumed cleanly.

Where one source record produces several Buzz events (a message and its
reactions), each event carries the `<source_ref>` of the record it derives from,
not of its parent.

### Client behaviour

Clients MAY use the tag to render imported content distinctly. Clients that do
SHOULD make clear that the content originated elsewhere without implying the
signing key is the original author — see Non-Goals.

Clients MUST tolerate:

- an unrecognised `<source>`, rendering a generic "imported" affordance or none;
- a `<source_ref>` whose structure they do not understand, treating it as opaque;
- events with no `imported_from` tag, which is the overwhelming majority.

Clients MUST NOT parse `<source_ref>` for any purpose other than display and
deduplication against a known `<source>`.

### Querying

The tag is queryable through the existing generic tag filter. Buzz indexes
`events.tags` with a GIN index using `jsonb_path_ops`
(`migrations/0004_events_tags_gin.sql`), so JSONB containment against this tag
is an index probe:

```sql
SELECT 1 FROM events
WHERE tags @> '[["imported_from","slack","T024BE7LH/C0GENERAL/1709545400.000400"]]'
```

This is the idempotency check an importer needs, and it needs no new index.

A containment probe on the prefix alone matches every event from a source:

```sql
SELECT count(*) FROM events WHERE tags @> '[["imported_from","slack"]]'
```

JSONB array containment is superset semantics, so a two-element probe matches
the three-element tag — the same property documented for the `shared` tag in
`crates/buzz-db/src/event.rs`, where a two-element probe is noted as also
matching `["shared","true","extra"]`.

For `shared` that is a hazard, and ingest guards it with an exact-shape
`parts.len() == 2` check so the pushdown stays sound. For `imported_from` it is
the desired behaviour: it is what makes the prefix query above work without a
second index. The corollary is that a containment probe alone cannot assert the
tag's exact arity. Anything that needs that — a relay policy keyed on
provenance, for example — would need an ingest-side shape check of the kind the
`shared` tag already has. This NIP deliberately does not ask for one, because
nothing here is a security boundary (see Non-Goals).

## Rationale for a tag rather than a kind

Buzz's `AGENTS.md` directs new features toward new kinds rather than new HTTP
endpoints, and that guidance is followed elsewhere in this repo. It does not
apply here, for two reasons.

Provenance is an **attribute of an existing event**, not an operation. An
imported message is still a kind:9 stream message; it must appear in channel
windows, thread pagination, search and notification fan-out exactly as any other
message does. Giving it a distinct kind would require every read path, filter,
and client to learn a parallel kind for content that behaves identically —
precisely the divergence new kinds are meant to avoid.

A separate provenance *event* referencing the imported one is the other
alternative. It doubles event count for the whole import, requires a join on
every read that wants the badge, and can be lost independently of what it
describes. A tag is atomic with the event it describes and costs nothing to
carry.

## Reference

The convention as specified here is implemented by
[slack2buzz](https://github.com/Ferchmin/slack2buzz), a Slack archive importer.
