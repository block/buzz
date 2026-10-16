# Bridge current audience authority

`POST /query` supports a bounded authorization check for shared private sessions.
Unlike discovery events, its decisions read current channel and membership rows
from one writer database statement. It does not return session content.

Resolved-community NIP-11 documents advertise
`audience_authority: {"version":1,"community_id":"<uuid>","max_sources":100}`.

Send one filter, authenticated through the normal HTTP bridge admission path:

```json
[{"kinds":[39000,39002],"consistency":"strong","audience_authority":{"version":1,"destination":"<uuid>","sources":["<uuid>"]}}]
```

Sources must be 1–100 distinct canonical lowercase UUIDs. Extra fields, mixed
filters, unsupported versions and malformed IDs are rejected with HTTP 400.
The response is an object, never an ordinary event array:

```json
{"audience_authority":1,"complete":true,"community_id":"<uuid>","pubkey":"<authenticated hex>","destination":"<uuid>","sources":[{"id":"<uuid>","allowed":true}]}
```

Decisions preserve request order. Every source receives a boolean, including
missing or inaccessible sources, without disclosing denial reasons or rosters.
The tenant comes from resolved host routing, and the viewer from authentication.

An allowed source must currently be a nondeleted private stream with a valid
`Buzz session (buzz.sessions/v1)` description (optional canonical parent UUID
and personal suffix). The destination must exist, be nondeleted and private.
The viewer must be an active member of both channels, and every active
destination member must be an active source member. Coverage uses `NOT EXISTS`
over membership rows, without a roster-size truncation. Archived channels remain
readable; hidden placement does not revoke membership. Public destinations
cannot be covered by these private sessions.

Clients must bind the response to the authenticated viewer, expected destination,
exact requested source IDs and community from trusted connection discovery.
Unsupported servers, ordinary event arrays, incomplete/malformed responses and
errors must fail closed. Run the check after content/title reads and before
returning protected data; do not cache it as a permission grant. A successful
decision describes the final database snapshot, not a lease on future access,
and cannot retract content a model has already consumed.

Database regressions exercise the real authenticated router while committed
membership changes leave signed discovery events unchanged. This models delayed
or failed publication; it does not inject a failure into the publication worker.
