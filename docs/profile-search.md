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


## Operator-delivered verified names

`POST /operator/communities/verified-profile` lets a trusted operator deliver a
current, issuer-signed human name assertion without waiting for the target device
to reconnect. It produces the same relay-signed kind-30382 used by profile search.
It never creates kind-0 metadata or signs as the target, and does not enroll keys,
grant access, change membership, or replace user-selected names or avatars.

The request uses the deployment operator's payload-bound NIP-98 signature against
`RELAY_OPERATOR_API_ORIGIN`; the existing operator allowlist and replay protection
apply. Its JSON body contains exactly:

```json
{
  "community_id": "<community UUID>",
  "expected_host": "workspace.example.org",
  "assertion": "<current human NIP-FI compact JWT>"
}
```

The UUID and canonical host must resolve to the same active community served by
this relay. The management request's inbound Host does not select a tenant. The
canonical assertion verifier checks issuer trust, signature, audience, token
class, key claims and validity bounds against that community. Only the verified
`nostr_pubkey` and validated `nostr_public_profile` supply the target and names;
operator-selected names or key overrides are rejected. The assertion must be for
a human, not an agent or OAuth client subject. Display names may contain internal
spaces and valid NFC Unicode under the existing 64-character claim rules.

Publication requires a stable relay key, global NIP-FI enforcement and the
issuer's enabled public-profile policy. The shared issuer/key deny map and
community bans apply. The target must already be a relay member, even when
ordinary relay admission does not require explicit membership. The operator's
signature is proof of delivery, not proof of possession of the target key.
`verified_at` records assertion verification; it does not claim the device
connected at that time.

A committed write responds with `{"status":"published","event":{...}}`. Verify
the returned relay signature and the event's `p`/`d` target tags when checking a
consumer integration. Storage failure returns 503 `publication_unavailable`.
`{"status":"not_written"}` means no new event was written: the assertion may be
unchanged, superseded, or no longer eligible. It is not confirmation that the
submitted name is current. Delivery to subscribers is best effort after durable
storage; consumers can fetch the stored event if they miss that notification.

Within one issuer, an older `iat` cannot replace the current snapshot. Different
names with an equal `iat` are ambiguous and do not replace it either; obtain a
strictly newer assertion to change the names. An identical assertion may still
refresh after the configured interval or adopt a changed public issuer label.
Ordering is assertion issuance order, not an upstream directory version. The
producer must obtain fresh authoritative names, serialize/version its source
updates and retry with newly signed NIP-98 requests. Replaying an old assertion
does not discover a new directory name.

This endpoint has no roster consumer, scheduled backfill, issuer-token minting or
automatic publication at provisioning time. A provisioning or roster worker must
have an independently authorized way to obtain the current human assertion for
each currently verified device key. A cached key receipt alone is insufficient.
Do not use server-derived replacement identities or retain employee private keys
for this purpose. Existing accounts can be explicitly reconciled through this
endpoint once those prerequisites hold. The record retains the existing
historical verified-name semantics; revocation stops eligibility/access rather
than deleting historical names. Clients must consume kind-30382 separately from
employee-authored kind-0 and retain user-profile precedence. Older clients that
read only kind-0 will not gain name display or search from this endpoint alone.
