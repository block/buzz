# Identity ownership preview

An authenticated deployment operator can inspect an exact key before archiving
it and its directly owned agents.

```http
GET /api/admin/v1/members/<hex-pubkey>/ownership?communityHost=community.example
Authorization: Nostr <NIP-98 event>
```

Sign the full admin-origin URL, including the community query and actual HTTP
method. This route requires the Operator role even when other moderation reads
are network-trusted; Moderators and unsigned callers cannot use it.

The same preview is served at
`GET /operator/communities/members/<hex-pubkey>/ownership?communityHost=…` for
an operator in `RELAY_OPERATOR_PUBKEYS`, signed against
`RELAY_OPERATOR_API_ORIGIN` (see [Operator API
routes](operator-identity-archive.md#operator-api-routes)).

The response contains:

- `communityId`, `communityHost`, and the requested canonical `pubkey`;
- `identities`: the root key first, followed by directly owned agents from the
  relay's verified, community-local ownership mapping. Each row has `pubkey`,
  optional `displayName` and `ownerPubkey`, and the current NIP-IA
  `archived` state;
- `workflows`: metadata for workflows owned by those exact keys, including
  `id`, `name`, `ownerPubkey`, optional `channelId`, saved `status` and
  `enabled`, and `pendingRuns`, `runningRuns`, and `waitingApprovalRuns` counts;
- `identitiesTruncated` and `workflowsTruncated` flags. Each collection is capped
  at 200 rows; workflows cover only the identities returned. A truncated identity
  list therefore also makes the overall workflow inventory incomplete;
- `inventoryVersion`: a digest of the identity keys, or `null` when the
  identity list is truncated. The workflow preview is informational.

All facts come from one read-only, repeatable-read database snapshot. The root
is returned even if it has no profile. An empty child list means there are no
materialized ownership links in that snapshot; it does not establish the absence
of issuer-only agents, disconnected hosts, or external automation.

The preview provides workflow metadata. Definitions, trigger contexts,
approval tokens, credentials, and private profile configuration are outside
its scope. The archive request echoes `inventoryVersion`. If the relay's
re-read returns different keys, the caller must confirm an updated preview.
See [Operator archive and unarchive](operator-identity-archive.md).
