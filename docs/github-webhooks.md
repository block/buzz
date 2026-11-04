# GitHub webhooks

A channel can follow a GitHub repository. The relay posts one message per new
pull request and keeps a relay-signed [NIP-AR](nips/NIP-AR.md) artifact with the
pull request's state, checks and reviews, so every client can show a live card
without calling GitHub itself.

## Following a repository

A channel member publishes a `buzz.github.hook` artifact in the channel:

```json
{
  "kind": 45010,
  "tags": [
    ["ar", "1"], ["d", "<uuid>"], ["h", "<channel>"],
    ["type", "buzz.github.hook"], ["title", "owner/repo"], ["op", "create"],
    ["repo", "owner/repo"]
  ],
  "content": "{\"version\":1,\"repo\":\"owner/repo\",\"secret\":\"<NIP-44 v2 ciphertext>\"}"
}
```

`secret` is the webhook secret, NIP-44 encrypted from the author's key to the
relay's pubkey (NIP-11 `self`). Channel readers see only ciphertext. The author
then adds a webhook in the repository's settings:

- Payload URL: `https://<relay host>/hooks/github/<d>`
- Content type: `application/json`
- Secret: the plaintext secret
- Events: Pull requests, Pull request reviews, Check suites, Check runs, Statuses

To rotate the secret, update the artifact. To stop following, delete it.

Each delivery acts with the hook author's current authority: the relay rejects
it with `403` once the author can no longer post in the channel, including
while they are timed out. Until the `X-Hub-Signature-256` signature verifies,
every failure answers `404`. A delivery for another repository, as an
organization webhook sends, answers `202` and changes nothing.

## Pull request artifacts

The first `pull_request` delivery for a pull request, of any action except
`closed`, creates a `buzz.github.pull` artifact, then posts `#<number> ·
<title>` and the pull request URL, and roots the artifact on that message.
Pull requests opened before the hook appear on their next activity. Only the
relay signs these; clients trust a revision only when its `pubkey` is the
relay's. Its `d` is derived from a relay secret, so no one else can claim it
first.

NIP-AR lets any channel writer revise an artifact. When a member edits,
moves or deletes a card, the relay's next write for that pull request replaces
it with the relay's own latest state, restoring or moving it back as needed.
A card whose current revision a moderator redacts stays retired. The relay
handles one delivery at a time per pull request.

| Tag | Value |
| --- | --- |
| `r` | Pull request URL |
| `repo` | `owner/repo`, lowercase |
| `hook` | The hook's `d` |
| `sha` | Head commit SHA |
| `state` | `open`, `draft`, `merged` or `closed` |

Content (version 1):

```json
{
  "version": 1, "url": "https://github.com/o/r/pull/7", "repo": "o/r", "number": 7,
  "title": "…", "author": "login", "state": "open",
  "base": "main", "head": "branch", "head_sha": "…", "updated_at": "2026-11-04T10:00:00Z",
  "additions": 3, "deletions": 1, "changed_files": 2,
  "checks": {"state": "pending", "runs": {"buildkite": "success", "lint": "pending"}},
  "reviews": {"carol": "approved"},
  "requested_reviewers": ["dan", "team:docs"]
}
```

- `checks.state` is `none`, `pending`, `success` or `failure`. `runs` is keyed by
  check suite app slug, check run name or commit status context, and resets
  when the head SHA changes.
- `reviews` keeps each reviewer's latest approval or change request; a comment
  doesn't replace it, and a dismissal does.
- A delivery older than the stored `updated_at` doesn't change the stored
  pull request fields.

The relay replies in the pull request's thread when it is merged, closed,
reopened or marked ready for review, and when a reviewer approves or requests
changes. A redelivered event repeats no reply unless a later one changed the
same state in between. `check_run` and `check_suite` deliveries without pull
requests, as GitHub sends for forks, find their cards by head SHA. A check for
a commit the card doesn't show yet is not recorded.

## Deployment

GitHub must be able to reach `POST /hooks/*` on the relay without other
authentication in front of it.
