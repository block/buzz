# Identity ownership preview

An authenticated deployment operator can inspect an exact key before retiring
an identity and its automation:

```http
GET /api/admin/v1/members/<hex-pubkey>/ownership?communityHost=community.example
Authorization: Nostr <NIP-98 event>
```

Sign the full admin-origin URL, including the community query and actual HTTP
method. This route requires the Operator role even when other moderation reads
are network-trusted; Moderators and unsigned callers cannot use it.

The response contains:

- `communityId`, `communityHost`, and the requested canonical `pubkey`;
- `identities`: the root key first, followed by directly owned agents from the
  relay's verified, community-local ownership mapping. Each row has `pubkey`,
  optional `displayName` and `ownerPubkey`, current NIP-IA `archived` state,
  `executionHeld` (the key has its own identity-retirement execution hold), and
  `ownerExecutionHeld` (the key's recorded owner is held, which also blocks the
  key's new work; `false` for a key with no recorded owner). A key is blocked
  when either is `true`. Both are always present: a client that cannot read
  them must treat the key's state as unknown, not as not held;
- `workflows`: metadata for workflows owned by those exact keys, including
  `id`, `name`, `ownerPubkey`, optional `channelId`, saved `status` and `enabled`,
  and `pendingRuns`, `runningRuns`, `waitingApprovalRuns` and `drainingRuns`
  (cancelled runs whose executor has not yet confirmed it stopped) counts;
- `identitiesTruncated` and `workflowsTruncated` flags. Each collection is capped
  at 200 rows; workflows cover only the identities returned. A truncated identity
  list therefore also makes the overall workflow inventory incomplete;
- `inventoryVersion`: a digest of the identity keys and each workflow's id,
  owner, status and enabled flag, or `null` when either list is truncated.

All facts come from one read-only, repeatable-read database snapshot. The root
is returned even if it has no profile. An empty child list means there are no
materialized ownership links in that snapshot; it does not establish the absence
of issuer-only agents, disconnected hosts, or external automation.

This endpoint neither archives identities nor pauses workflows or processes.
It does not return workflow definitions, trigger contexts, approval tokens,
credentials, or private profile configuration. Execution hosts, local timers and
watches, credential/delivery registrations, and shared-resource dependencies are
outside this preview. `inventoryVersion` is not an authorization: an archive
request echoes it so the relay can refuse when its own re-read differs (see
[operator-identity-retirement.md](operator-identity-retirement.md)). A truncated
inventory has no version and cannot be retired.
