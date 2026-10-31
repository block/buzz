NIP-CE
======

Custom Emoji Entries
--------------------

`draft` `optional` `relay`

This NIP defines `kind:30180`, an addressable event that holds one custom emoji. A community can hold many thousands of emoji without any client loading the whole catalog.

[NIP-30](30.md) and [NIP-51](51.md) put many emoji in one `kind:30030` set. A set is one event, so every client must read and parse the whole set. This NIP uses one event for each emoji. Clients ask the relay for the names they need.

## Event

`kind:30180` is an addressable event keyed by `(pubkey, kind, d)` as defined in [NIP-01](01.md).

```jsonc
{
  "kind": 30180,
  "content": "party parrot",
  "tags": [
    ["d", "party_parrot"],
    ["emoji", "party_parrot", "https://example.com/media/9f2c.gif"]
  ]
}
```

### Tags

An entry MUST have exactly one `d` tag and exactly one `emoji` tag.

The `emoji` tag has the form `["emoji", <shortcode>, <url>]`, as in [NIP-30](30.md).

The shortcode MUST be 1 to 64 bytes of ASCII letters, digits, hyphens (`-`), and underscores (`_`). It MUST be lowercase. The `d` value MUST equal the shortcode in the `emoji` tag.

The URL MUST start with `http://` or `https://`. It MUST NOT be longer than 2048 bytes.

An entry MUST NOT have an `h` tag. Entries are never scoped to a channel.

### Content

`content` is the search text. It is the shortcode with each `_` and `-` replaced by a space. For `party_parrot-2` the content is `party parrot 2`.

Full-text search splits on spaces. The replacement lets a search for `parrot` find `party_parrot`.

`content` MUST equal this search text exactly. A relay MUST reject an entry with any other `content`, including empty content. Because the content comes from the shortcode, it is never longer than 64 bytes.

The rule keeps the full-text index in step with the shortcode. An author cannot add keywords that the shortcode does not contain.

## Replace semantics

A new entry with the same `(pubkey, d)` replaces the old entry. The relay keeps the entry with the newest `created_at`. When two entries have the same `created_at`, the relay keeps the entry with the lowest event ID. This is the standard [NIP-01](01.md) rule.

A member can only replace their own entries. Another member who publishes the same `d` creates a second, separate entry. The relay stores both.

When clients see several entries for one `d`, they MUST pick the entry with the newest `created_at` across all authors. Clients SHOULD report the conflict to the user. Clients MUST apply the same tie-break as the relay.

To remove an entry, the author sends a [NIP-09](09.md) deletion request with exactly one `a` tag, `30180:<author pubkey>:<d>`. A deletion request names one entry, so removing many entries takes one request each. The relay removes the versions of that entry created at or before the request's `created_at`, so a later replacement stays.

## Reading

### Exact lookup

To resolve names, send a filter with `kinds` and `#d`:

```jsonc
{ "kinds": [30180], "#d": ["party_parrot", "sad_parrot"] }
```

A filter SHOULD NOT carry more than 50 names. A client that knows the author SHOULD add `authors`. Without `authors`, a relay can scan every entry of the kind to answer.

### Prefix search

To search, use [NIP-50](50.md) search over HTTP `POST /query` with prefix mode:

```jsonc
{ "kinds": [30180], "search": "parr", "search_mode": "prefix", "limit": 50 }
```

Prefix mode treats each word except the last as a whole word. It treats the last word as a prefix. A search for `party par` finds `party parrot`. A search for `joy` finds `bufo joy`.

The relay returns at most 500 hits for each request. Clients SHOULD ask for 50 or fewer.

The relay reads `search_mode` on `POST /query` only. A WebSocket `REQ` with `search` uses full-word matching.

Search needs the relay to index `content` of `kind:30180` in its full-text index. A relay that indexes only an allowlist of kinds MUST add `30180` to the allowlist before clients rely on search. Exact lookup does not depend on the full-text index.

In Buzz, a database that migration `0008_fresh_install_search_allowlist.sql` built, or that an operator converted with `scripts/maintenance/nip_rs_search_allowlist.sql`, indexes only an allowlist of kinds. Such a database does not index `kind:30180` until an operator runs the updated maintenance script. The operator MUST run it before the emoji migration rollout. Without it, entries are stored and exact lookup works, but prefix search returns no results.

## Relay behavior

A relay that supports this NIP:

- MUST accept `kind:30180` from any community member with the same permission as `kind:30030`.
- MUST reject an invalid entry with an `invalid:` message.
- MUST store entries as global events, not channel events.

## Relationship to kind 30030

This NIP does not change `kind:30030`. Sets with `d` equal to `buzz:custom-emoji` keep their meaning. A client MAY read both. A client that reads both SHOULD prefer the newest event when a shortcode appears in both.

The `emoji` tags on messages and reactions do not change. A message carries the `emoji` tags for the emoji it uses, so history renders without any lookup.

Clients that do not know `kind:30180` ignore these events.

## Limits summary

| Item | Limit |
|------|-------|
| Shortcode | 1 to 64 bytes, `[A-Za-z0-9_-]`, lowercase in `d` and `emoji` |
| URL | 2048 bytes, `http://` or `https://` |
| Content | 256 bytes |
| `d` tags | exactly 1 |
| `emoji` tags | exactly 1 |
| `h` tags | 0 |
| Names per `#d` filter | 50 (SHOULD) |
| Search hits per request | 500 (relay maximum) |
