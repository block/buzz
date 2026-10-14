# Profile name search

`POST /profiles/search` finds community members by profile name. It matches
only the `name` and `display_name` string fields of each current global kind:0
profile. It does not search `picture` (including data URLs), `about`, `nip05`,
`banner`, or any other profile field. NIP-50 search over kind:0 matches the
whole profile JSON, so it can return profiles whose avatar data or bio happens
to contain the query text.

Apart from the fields it reads, the endpoint matches and pages like a NIP-50
kind:0 search with `search_mode: "prefix"`. A client can switch from one to the
other without a visible change in which people a query finds.

The endpoint requires NIP-98 authentication over the request body and relay
membership, like other relay-owned member endpoints. A relay without the
endpoint answers `404`; clients fall back to NIP-50 search for that request.

Request:

```json
{ "query": "wes", "limit": 20, "page": 1 }
```

- `query`: 1 through 100 characters after trimming. Required.
- `limit`: results per page, 1 through 100. Defaults to 20.
- `page`: 1-based page of `limit` results, 1 through 1000. Defaults to 1.
- Unknown fields are rejected with `400`.

Response: a JSON array of signed kind:0 events for that page, best match
first. A page shorter than `limit` is the last page.

Matching is the NIP-50 prefix rule, case-insensitive. The query is split on
whitespace. Every word except the last must equal a word of `name` or
`display_name`; the last word may be a word prefix. Words may appear in any
order and may be split across the two fields, so "johnson wes" finds
"Wes Johnson". Accents are not folded.

Results are ordered:

1. `name` or `display_name` equal to the query;
2. a name that starts with the query;
3. a name with a later word that starts with the query;
4. any other match.

For these tiers the query and both names are lower-cased, with whitespace
runs collapsed to one space and trimmed, by the same database expression.
Ties go by relevance of the query to the two names alone (not the whole
profile, as in NIP-50), then newest first, then event id, so pages do not
overlap. Profile content that is not a JSON object, or a name field that
is not a string, never matches. Content that PostgreSQL cannot read as `jsonb`
(for example a `\u0000` escape or very deep nesting) has no names, so it
never matches and never makes the search fail
(`schema/functions/profile_name_fields.sql`).
