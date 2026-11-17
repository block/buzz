# Managed community profile details

A configured identity issuer may include the optional `nostr_managed_profile`
object in a human authentication assertion alongside the existing, closed
`nostr_public_profile` name claim. It is display data, never authorization.
The relay publishes it only after successful admission, under the same
community, authenticated issuer, subject and proven public key as the names.
Agents do not receive managed profiles.

```json
{
  "version": 1,
  "source_label": "Example Directory",
  "source_url": "https://example.org/people/alice",
  "fields": [
    {"key": "team", "section": "team", "label": "Team", "value": "Design"},
    {"key": "lead", "section": "team", "label": "Led by", "value": "Bob Example", "username": "bob"}
  ],
  "city": "London",
  "timezone": "Europe/London",
  "picture": "https://example.org/people/alice.png"
}
```

The object is limited to 16 KiB of UTF-8 JSON and 16 uniquely keyed fields.
Keys and related usernames are lowercase ASCII handles, up to 64 characters.
Sections are `personal`, `team`, or `location`. Source labels and field labels
are limited to 64 Unicode characters; values to 512; city/timezone to 128.
Text must be trimmed and contain no control characters. URLs are HTTPS without
embedded credentials, up to 2048 bytes. Clients must validate timezones before
using them. Unknown fields, duplicate keys, unsupported versions and malformed
optional data are omitted without failing authentication.

## Publication and compatibility

The existing kind **30382** remains unchanged: its content is empty and its
name tags retain their closed format. NIP-11's
`federated_identity.public_profile.managed_kind: 30383` advertises a companion
kind **30383** with the same subject/name/issuer-label/verification tags and
managed JSON as content. Empty content means no managed snapshot. Both kinds
are relay-only and are stored atomically; both are returned by profile search.
Neither event reveals the private issuer URL or issuer subject.

Clients supporting managed data subscribe to **both** kinds, allowing existing
30382-only profiles to remain verified until their next authentication. The
companion wins at equal timestamps. Legacy clients continue reading30382.
A relay upgrade does not backfill or fetch directory data; an issuer supplies
it during later authentication refreshes.

An omitted or invalid optional claim preserves a saved snapshot only when the
issuer, private subject and verified username remain the same. Changing any
of them clears the retained snapshot. A successful empty directory snapshot
(`fields: []`, no city/timezone/picture) replaces previously populated values.
Older assertions cannot overwrite a newer head. The name verification time
records admission; it does not claim that every retained directory value was
fetched at that instant.

Managed fields stay outside user-authored kind0 metadata. A suggested picture
is a default only: clients must preserve a chosen photo and explicit removal.
The generic client marks those decisions with `managed_photo_initialized` in
kind0. It can import a default picture once and use managed timezone directly.
`username` and `timezone` field keys duplicate canonical profile rows, so the
client shows them once while retaining them in its managed editor section.

Roll out the relay migration/support first, then supporting clients and an
opt-in issuer. Removing issuer enrichment alone preserves existing snapshots;
issue an explicit empty snapshot to clear directory details.
