# Webhook direct messages

A channel owner or admin can create a webhook workflow that sends a private
message from the workflow owner to a recipient supplied by the caller:

```yaml
name: Review notifications
trigger:
  on: webhook
steps:
  - id: notify
    action: send_dm
    to: '{{trigger.recipient}}'
    text: 'Review requested: {{trigger.title}} — {{trigger.url}}'
```

POST JSON to `/hooks/<workflow-id>` with the workflow's `X-Webhook-Secret` header.
Never put the secret in a query parameter, where URLs may expose it in logs:

```json
{
  "recipient": "alice@chat.example.org",
  "title": "Checkout",
  "url": "https://reviews.example.org/42"
}
```

`recipient` must be a 64-character hex public key or an existing NIP-05 handle
registered in the workflow's community. Handles are case-insensitive and must
use that community's host; the relay never performs external NIP-05 discovery.
These are community profile handles, not verified email addresses or external
directory identities. Integrations must establish their own mapping from an
external account to a public key or local handle; never derive that mapping from
a display name or assume an email address identifies the same person. Prefer a
pinned public key when an integration needs a stable identity across handle changes.

The owner explicitly delegates recipient selection to holders of the webhook
secret by choosing `{{trigger.recipient}}`. Protect the secret like a credential
that can send DMs as the owner. A fixed public key in `to` keeps recipient selection
under the workflow owner's control. Literal handles in `to` are rejected because handles can be reassigned;
only the explicit webhook recipient template accepts a handle at run time.

Only webhook definitions and webhook-caused runs may use this recipient template.
Other arbitrary templates, concatenation, and step-output recipients are rejected.
`{{trigger.author}}` retains its event-author meaning; a webhook JSON field named
`author` does not populate it. Payload values are not recursively templated.
Missing, malformed, unknown, banned, off-community, and self recipients fail the
run without creating a conversation. On members-only communities, the recipient
must also be a current relay member, or an agent whose recorded owner is a current
member. Removing that membership revokes eligibility; a retained profile does not
restore it. Open communities do not require relay membership. Existing private
conversations are reused without changing their hidden state. Owner authority and agent-mention policy
are checked as for other workflow DMs.

HTTP 202 means the run was accepted, not that the message arrived. Check the run
trace for completion and its message event ID. Retrying the webhook starts a new
run and may send a duplicate; delivery is not exactly once. There is no per-secret
run or recipient limit. Integrations should rate-limit and deduplicate before
calling the webhook; use a pinned public key if a caller must have one destination.

## Manual verification

1. As a channel owner/admin, save the workflow above and get its webhook secret.
2. POST a real local recipient's public key, then their local NIP-05 handle.
   Both runs should complete and their signed messages should appear in the same
   private conversation with the workflow owner.
3. Try a missing recipient, unknown handle, off-community handle, and banned
   recipient. The run should fail and no new conversation should appear.
4. Include an `author` field in the JSON. It must not change the sender or the
   event-author context. Changing the workflow trigger to `message_posted`
   must make `{{trigger.recipient}}` invalid.
