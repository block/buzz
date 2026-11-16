NIP-CE
======

Custom Emoji Entries
--------------------

`draft` `optional` `relay`

This NIP defines `kind:30180`, an addressable event that holds one custom emoji, and the catalog a relay builds from these events. A community's catalog has one emoji for each shortcode and can hold 100,000 or more.

[NIP-30](30.md) and [NIP-51](51.md) put many emoji in one `kind:30030` set. A set is one event, so every client must read and parse the whole set. This NIP uses one event for each emoji. Clients keep a local copy of the catalog and ask the relay only for what changed.

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

An entry MUST have exactly one `d` tag and exactly one `emoji` tag. A [removal](#removal) has no `emoji` tag.

The `emoji` tag has the form `["emoji", <shortcode>, <url>]`, as in [NIP-30](30.md).

The shortcode MUST be 1 to 64 bytes of ASCII letters, digits, hyphens (`-`), and underscores (`_`). It MUST be lowercase. The `d` value MUST equal the shortcode in the `emoji` tag.

The URL MUST start with `http://` or `https://`. It MUST NOT be longer than 2048 bytes.

An entry MUST NOT have an `h` tag. Entries are never scoped to a channel.

### Content

`content` is the search text. It is the shortcode with each `_` and `-` replaced by a space. For `party_parrot-2` the content is `party parrot 2`.

Full-text search splits on spaces. The replacement lets a search for `parrot` find `party_parrot`.

`content` MUST equal this search text exactly. A relay MUST reject an entry with any other `content`, including empty content. Only a [removal](#removal) has empty content. Because the content comes from the shortcode, it is never longer than 64 bytes.

The rule keeps the full-text index in step with the shortcode. An author cannot add keywords that the shortcode does not contain.

## The catalog

A relay keeps one catalog for each community. The catalog has at most one emoji for each shortcode.

### Ownership

A shortcode belongs to the member whose entry first claimed it. That member is its owner.

- The owner MAY publish a new entry for the shortcode. It replaces the image.
- A community owner or admin MAY publish an entry for any shortcode. They become its owner, and the relay retires the previous owner's entry event.
- A relay MUST reject an entry from any other member with an `invalid:` message. The catalog does not change.

Between entries from one author for one shortcode, the standard [NIP-01](01.md) rule applies: the relay keeps the entry with the newest `created_at`, and the lowest event ID on a tie. A relay MUST reject an entry that loses to one its author already published, so the author knows the catalog did not change. To replace or remove an emoji added in the same second, the author publishes with a later `created_at`.

The relay keeps one live `kind:30180` event for each shortcode: the event of its current owner.

### Removal

A removal is a `kind:30180` event with one `d` tag, no `emoji` tag, and empty content:


```jsonc
{
  "kind": 30180,
  "content": "",
  "tags": [["d", "party_parrot"]]
}
```


The owner of a shortcode, or a community owner or admin, MAY remove it. A relay MUST reject a removal for a shortcode that has no emoji.

After a removal the shortcode is free. The next member to publish an entry for it becomes its owner.

A relay MUST reject a [NIP-09](09.md) deletion request that targets a `kind:30180` event. A removal is the only way to take an emoji out of the catalog, so every change to the catalog is itself an entry.

A relay that deletes an entry event by moderation MUST remove the emoji from the catalog in the same step, as a new change.

Ownership follows the key that signs the entry. An agent's entries belong to the agent, not to the agent's owner.

Messages and reactions that used a removed emoji still render. They carry their own `emoji` tags.

### Change numbers

Every entry or removal the relay accepts gets the next change number of its community. Change numbers start at 1 and only grow. Each shortcode keeps the number of its latest change.

A client that holds every change up to `n` is up to date after it reads the changes above `n`. The relay commits changes in number order, so a client never misses a change that has a lower number than one it already read.

## Reading

### Catalog changes

`POST /emoji/catalog` reads the catalog. The request uses the same [NIP-98](98.md) authorization as `POST /query`, and the caller MUST be a member of the community.

To read changes, send the last change number the client holds. Send 0 to read the whole catalog.


```jsonc
{ "since": 41, "limit": 10000 }
```


`limit` is optional. It is 1 to 10000 and defaults to 10000.

The response lists the shortcodes that changed after `since`, lowest change number first:


```jsonc
{
  "emoji": [
    ["party_parrot", 42, "https://example.com/media/9f2c.gif", 0],
    ["sad_parrot", 43, null, 1]
  ],
  "authors": ["<64-hex pubkey>", "<64-hex pubkey>"],
  "more": false
}
```


Each row is `[shortcode, change, url, author]`. `url` is `null` when the shortcode was removed. `author` is an index into `authors`: the owner of the shortcode, or the member who removed it.

When `more` is `true`, the client reads again with `since` set to the change number of the last row.

A read with `since` 0 leaves out removed shortcodes. A client that reads from 0 MUST replace its local copy with the result.

The relay MAY compress the response. Rows are compact because a catalog of 100,000 emoji is read in full by every new client. Signed events for the same catalog are about five times larger.

Clients SHOULD read changes when they connect and when the user opens an emoji picker. Clients SHOULD NOT poll on a timer shorter than 15 minutes.

### Exact lookup

To resolve names without a local copy, send the names to the same endpoint:


```jsonc
{ "shortcodes": ["party_parrot", "sad_parrot"] }
```


A request carries 1 to 100 names. The relay normalizes each name the way [NIP-30](30.md) shortcodes are normalized here: it removes surrounding colons and lowercases. The response has the same shape. It holds a row for each name that has an emoji, and `more` is `false`.

A client with a local copy SHOULD look a name up this way when the copy does not have it, before it treats the name as unknown.

### Events

Entries are also ordinary events. A filter with `kinds` and `#d` returns the live event for a shortcode:


```jsonc
{ "kinds": [30180], "#d": ["party_parrot"] }
```


A client that reads events MUST treat an event with no `emoji` tag as a removal.

### Prefix search

A client with a local copy searches it locally. Without one, use [NIP-50](50.md) search over HTTP `POST /query` with prefix mode:


```jsonc
{ "kinds": [30180], "search": "parr", "search_mode": "prefix", "limit": 50 }
```


Prefix mode treats each word except the last as a whole word. It treats the last word as a prefix. A search for `party par` finds `party parrot`. A search for `joy` finds `bufo joy`.

The relay returns at most 500 hits for each request. Clients SHOULD ask for 50 or fewer.

The relay reads `search_mode` on `POST /query` only. A WebSocket `REQ` with `search` uses full-word matching.

Search needs the relay to index `content` of `kind:30180` in its full-text index. A relay that indexes only an allowlist of kinds MUST add `30180` to the allowlist before clients rely on search. Catalog reads do not depend on the full-text index.

In Buzz, a database that migration `0008_fresh_install_search_allowlist.sql` built, or that an operator converted with `scripts/maintenance/nip_rs_search_allowlist.sql`, indexes only an allowlist of kinds. Such a database does not index `kind:30180` until an operator runs the updated maintenance script. Without it, entries are stored and catalog reads work, but prefix search returns no results.

## Relay behavior

A relay that supports this NIP:

- MUST accept `kind:30180` from any community member with the same permission as `kind:30030`.
- MUST reject an invalid entry with an `invalid:` message.
- MUST apply each accepted entry to the catalog in the same transaction that stores the event.
- MUST store entries as global events, not channel events.

## Relationship to kind 30030

This NIP does not change `kind:30030`. Sets with `d` equal to `buzz:custom-emoji` keep their meaning. Emoji in sets are not in the catalog. A client MAY read both. A client that reads both SHOULD prefer the catalog when a shortcode appears in both.

The `emoji` tags on messages and reactions do not change. A message carries the `emoji` tags for the emoji it uses, so history renders without any lookup.

Clients that do not know `kind:30180` ignore these events.

## Limits summary

| Item | Limit |
|------|-------|
| Shortcode | 1 to 64 bytes, `[A-Za-z0-9_-]`, lowercase in `d` and `emoji` |
| URL | 2048 bytes, `http://` or `https://` |
| Content | 256 bytes |
| `d` tags | exactly 1 |
| `emoji` tags | exactly 1, or 0 in a removal |
| `h` tags | 0 |
| Rows per catalog page | 10000 |
| Names per lookup | 100 |
| Search hits per request | 500 (relay maximum) |
