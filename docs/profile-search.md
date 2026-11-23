# Profile name search

`POST /profiles/search` finds community members by profile name. It matches
only the `name` and `display_name` string fields of each current global kind:0
profile and, where the community advertises NIP-FI verified profiles
(`federated_identity.public_profile.searchable` in NIP-11), the
`display_name`, `username` and `full_name` tags of each current kind:30382
verified profile signed by the relay key. It does not search `picture`
(including data URLs), `about`, `nip05`, `banner`, or any other profile field. NIP-50 search over kind:0 matches the
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

Response: a JSON array of signed events for one page of people, best match
first. Each person contributes their current kind:0, when they have one, then
their current relay-signed kind:30382, when verified profiles are advertised
and they have one, whichever of the two matched. A page with fewer than
`limit` people is the last page. Clients that read only kind:0 keep working;
they also find people who match only by verified name.

Matching is the NIP-50 prefix rule, case-insensitive. The query is split on
whitespace. Every word except the last must equal a word of `name` or
`display_name`; the last word may be a word prefix. Words may appear in any
order and may be split across the two fields, so "johnson wes" finds
"Wes Johnson". A verified profile matches the same way over its three name
tags, except that its last word must be at least three characters, as in
NIP-50. The two sources match separately: a query whose words are split
between a person's kind:0 and their verified profile does not match.
Accents are not folded.

People are ordered by their best match across both sources:

1. a name equal to the query;
2. a name that starts with the query;
3. a name with a later word that starts with the query;
4. any other match.

For these tiers the query and the names are lower-cased, with whitespace
runs collapsed to one space and trimmed, by the same database expression.
Ties go by relevance of the query to the names alone (not the whole
profile, as in NIP-50), then newest first, then pubkey, so pages do not
overlap. Profile content that is not a JSON object, or a name field that
is not a string, never matches. Content that PostgreSQL cannot read as `jsonb`
(for example a `\u0000` escape), and content with more than 4096 `[` and `{`,
which bounds how deeply it can nest, has no names, so it never matches and
never makes the search fail (`PROFILE_NAME_FIELDS` in
`crates/buzz-search/src/query.rs`).

## Deployment

Verified-name matching reads `idx_nip_fi_profile_heads_names`. Relay startup
migration and `buzz-admin migrate` build it before migration `0081` under a
one-second lock and five-second statement budget. Applied directly as SQL,
`0081` builds it only while `nip_fi_profile_heads` is empty. A deployment that
applies migrations out of band (auto-migration off) builds it first, outside a
transaction:

```sql
CREATE INDEX CONCURRENTLY idx_nip_fi_profile_heads_names
    ON public.nip_fi_profile_heads USING GIN (
    to_tsvector('simple',
        COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "display_name")[1]', '{}', true) #>> '{}', '')
        || ' ' || COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "username")[1]', '{}', true) #>> '{}', '')
        || ' ' || COALESCE(jsonb_path_query_first(event, 'strict $.tags[*] ? (@[0] == "full_name")[1]', '{}', true) #>> '{}', ''))
);
```

If the build fails, drop the invalid index and build it again. The migration
then checks that the index is valid and has this definition.
