# Channel membership notices

The relay signs and stores channel system notices as kind `40099` with an `h`
tag naming the channel. A `member_joined` notice has JSON content:

```json
{
  "type": "member_joined",
  "actor": "<public key of the joining person or inviter>",
  "target": "<public key of the person added>",
  "role": "guest"
}
```

`role` records the effective role written by that membership operation:
`guest`, `member`, `bot`, `admin` or `owner`. It is historical display metadata,
not a grant of current authority. The existing member-add path also emits this
notice when changing an active member's role. Initial members supplied at channel
creation are admitted as `member`; the creator has a separate creation notice.

Clients describe Guest admission as “with view-only access”, Member and Bot as
“as a participant”, Admin as “as an admin”, and Owner as “as an owner”. Keep
inviter attribution for explicit additions. Roles must survive grouped notices;
do not assign one role to arrivals with different roles.

The field is optional for compatibility. Missing, unknown or malformed roles
retain generic wording; never infer a historical role from the current roster
or channel default. Older clients ignore the additional JSON field. No new tag,
event kind, migration or historical-event rewrite is required. Client visibility
policies still apply: a client that hides self-joins need not show them.
