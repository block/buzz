# Effective message edits

Text edits replace a message's complete attachment set. To retain files, a
writer needs the latest live edit's imeta, or the original's when no edit exists.
A plain `#e` query is an exact tag match and can miss valid historical targets
whose hex casing differs. This extension leaves generic NIP-01 matching intact.

## Request and response

An authenticated `POST /query` accepts one exact filter:

```json
[{"kinds":[9,40002],"ids":["<original event id, lowercase hex>"],"#h":["<channel UUID>"],"effective_edit":1,"consistency":"strong"}]
```

The version-1 envelope is:

```json
{"effective_edit":1,"original":{"id":"...","kind":9,"tags":[],"content":"...","pubkey":"...","created_at":0,"sig":"..."},"edit":null}
```

`original` and a non-null `edit` are complete signed Nostr events. `edit` is the
latest live kind 40003 in the same channel targeting the live kind 9/40002
original. Targets use the first valid 64-character hex `e` tag in signed order,
ignoring casing. The greatest `(created_at,id)` wins. An empty imeta set on that
edit is authoritative; it never falls back to original attachments.

Original liveness, fresh channel authorization, and edit selection share a
repeatable-read transaction on the primary. Ordinary per-event visibility gates
also apply. Missing and inaccessible targets both return the same 404. Lookup
failure returns an error, never an empty state. The normalized-target index from
migration 0086 bounds the latest-edit probe; unrelated channel edits are not read.

The CLI requires the versioned envelope, checks the returned target/channel, and
aborts without signing or publishing when an older relay returns an ordinary
array. Deploy the relay first, then release the CLI. Old clients retain their
existing behavior against the new relay. No schema change is needed beyond 0086.

The CLI stamps its replacement after the observed state, including same-second
edits. A head more than 60 seconds ahead of its clock causes an error until the
clock catches up. Concurrent writers still use `(created_at,id)` ordering; the
lookup does not create a compare-and-swap precondition on publication.
