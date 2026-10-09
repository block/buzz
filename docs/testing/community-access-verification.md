# Community access verification

This runbook verifies an already-deployed community-access revision. Preparing
this document does not establish a successful run, approve a deployment, or
complete verification. Use [the evidence template](community-access-evidence.md)
for each exact deployed revision.

## Prerequisites

1. Identify the deployment owner and obtain an authorized disposable community
   and dedicated test identities. Record setup and cleanup responsibility.
2. Invitation revocation and route coverage must be merged, required checks
   passing, and their source commits present in every serving revision under test.
   Include live-session revalidation and webhook-owner policy where required.
3. Record the immutable image digest, source SHA, serving pod/runtime revisions,
   deployment scope, UTC start time, and relevant configuration from the owner's
   deployment records. A merge SHA alone does not establish deployment.
4. Record membership requirements, NIP-FI mode, accessory API enablement, and
   any absolute v1 invitation cutoff. Include serving and rollback versions.
5. Keep private keys, auth tags, invite codes, webhook secrets, and request
   authorization headers in the client's credential store or process environment.
   Retain only sanitized statuses, event IDs, counts, timing, and fixture aliases.
   Disable shell tracing and do not export raw HAR files or app logs.
6. Read the route inventory at the deployed source revision. Map every enforced
   route to an evidence row. Record a reason, owner, and follow-up for each skip;
   unexplained or required skips prevent a passing verdict.

The route matrix and PostgreSQL test suites create fixtures directly in a
local database. Never run those suites against a hosted relay or database.
Staging uses client-visible actions and the authorized deployment owner's
read-only diagnostics. Setup authority is scoped to the disposable community.

## Local rehearsal

On the candidate branch, use the isolated local workflow in [TESTING.md](../../TESTING.md)
and run the route guard and complete affected suites before staging:

```sh
. ./bin/activate-hermit
python3 scripts/check-community-ban-route-inventory.py
cargo test --no-fail-fast -p buzz-db -p buzz-relay
scripts/postgres-test-run.sh
```

The guard is available on the route-coverage branch. Record the exact candidate
HEAD, database/Redis isolation, commands, outcomes, failures, and skips. These
results are source validation; retain staging evidence separately.

## Fixtures and baseline

Use two disposable communities, A and B, if tenant-isolation testing is
supported. Prepare aliases for an owner, an administrator, ordinary members,
an invitation issuer, fresh claimants, an owner-linked agent, a bystander,
and a harmless webhook workflow. Never target a real user's key.

For each identity, capture a baseline successful action before applying any
restriction. Verify root connection, a joined huddle, channel reads/writes,
invitation onboarding, and the applicable route controls. Record the actual
fixture aliases and community hosts locally; publish only sanitized evidence.

The existing CLI can sign moderation commands as the selected disposable owner.
Set BUZZ_RELAY_URL, BUZZ_PRIVATE_KEY, and any required BUZZ_AUTH_TAG for that
owner through the normal credential workflow before these commands:

```sh
buzz --format compact moderation ban --pubkey "$TEST_MEMBER_PUBKEY" --reason "disposable verification"
buzz --format compact moderation unban --pubkey "$TEST_MEMBER_PUBKEY"
buzz --format compact moderation timeout --pubkey "$TEST_MEMBER_PUBKEY" --expires-in 120
buzz --format compact moderation untimeout --pubkey "$TEST_MEMBER_PUBKEY"
```

Only run each command when that case calls for it. Record the accepted event ID
and owner-confirmed commit time; sending a command is not proof it committed.
Use the app for invite mint/claim and huddle actions. Keep an unrestricted
bystander connected throughout restriction cases.

## Invitation lifecycle

1. Mint durable v2 invitation I as the disposable issuer. Record issuance time,
   token format, expiry, and issuer alias without retaining the code.
2. Ban the issuer, confirm acceptance/commit, then attempt I with a fresh
   unrestricted claimant. Admission must fail; no membership may appear.
3. Unban the issuer and retry I. Permanently revoked v2 invitations stay denied.
   A newly minted v2 invitation must admit an unrestricted fresh claimant.
4. With another valid invitation, ban a dedicated claimant, then attempt claim.
   Admission must fail without requiring that claimant to already be a member.
   Repeat with an agent restricted through its owner where that mode is supported.
5. Verify allowed onboarding in community B independently of A's restriction.
6. For existing valid v1 invitations, unset cutoff permits redemption until the
   token's own expiry; claimant/owner restrictions still apply. V1 carries no
   issuer identity, so issuer-specific revocation cannot be demonstrated.
7. If an operator cutoff is configured, before-cutoff redemption is allowed,
   at/after-cutoff redemption is refused, earlier token expiry still wins, and
   v2 remains unaffected. Exercise boundary time cases in deterministic local
   tests; staging confirms the deployed configuration and observable current case.

Do not change a staging cutoff solely to run this document. A cutoff needs its
own deployment-owner authorization and consistent serving/rollback config.
Record the last possible v1 issuance across all serving and rollback versions.
The maximum natural drain is 30 days from that issuance, not from merge time.
If it is unknown or the drain is incomplete, full invitation closure remains
pending unless the responsible security owner approves a scoped decision.

## Ban and timeout entry points

For a member, administrator, and owner-linked agent, repeat baseline → ban →
denial → unban → allowed control. Cover the deployed inventory's applicable
HTTP event/query/count, media upload/download/HEAD, GIF, workflow-read, Git,
moderation, invite, root WebSocket, huddle, and accessory routes.

A banned administrator must not retain community access. Each request must
bind to the correct community host. Keep allowed controls paired with denials
so malformed input, missing resources, or unrelated auth errors cannot count
as proof of a ban gate. HEAD responses omit the body. Record the route's
expected status/error contract at the deployed revision.

Apply a timeout separately. Reads and already-admitted root/audio connections
must retain the documented policy; writes fail until expiry or untimeout.
A timeout is not a blanket ban. Repeat a same-key action in B to establish
that an A restriction did not cross the community boundary.

## Existing sessions and cross-pod evidence

1. Admit root and audio sessions before the restriction; include an owner-linked
   agent and an unaffected bystander. Record admitting runtime/pod identifiers.
2. Commit a ban or membership removal through its normal authorized operation.
   Measure from confirmed commit to each policy close with a monotonic timer.
3. Verify reconnect is denied and the bystander remains usable. For membership
   removal, enable only the existing policy's required membership mode.
4. If multiple pods serve the deployment, place existing sessions on different
   pods and perform the mutation on another pod. Record routing evidence and
   normal Redis fan-out results. Single-pod evidence proves only local behavior.
5. If BUZZ-271 is deployed, verify missed-delivery behavior in a local isolated
   rehearsal. In staging, a missed-delivery test requires the deployment owner's
   explicit, scoped fault-injection procedure; never disable shared Redis.
   Measure against the deployed documented staleness budget (30 seconds in the
   proposed implementation), including its load and database-failure assumptions.
6. Database failure/grace tests also belong in isolated rehearsal unless the
   owner authorizes a scoped fault. Record which guarantee is source-tested,
   runtime-tested, or unverified; do not infer cross-pod guarantees from a local close.

Removal/re-addition and unban use the authoritative state at revalidation. A
transient removal restored before a scan need not disconnect an existing
session. Record the committed ordering rather than implying historical-state
revocation. Mixed versions need their own limitation and rollout decision.

## Webhook owner policy

Create a harmless workflow with a secret and an allowed owner. A valid trigger
must create one run. A wrong secret must create none. Ban the owner and trigger
again: the expected generic denial creates no new run. Unban and confirm a new
run can be admitted. Repeat an allowed timeout-only owner control and a B control.
Use owner-authorized run-list reads to compare counts; do not publish the secret.

The admission check is point-in-time: a ban committed after the owner's final
restriction read may race with insertion. Already-created runs are unaffected.
This case does not alter scheduled or event-triggered workflow policy. A lookup
failure must deny without enqueueing; use the isolated local failure harness.

## Cleanup and verdict

Lift fixture bans/timeouts, end test sessions, delete/revoke remaining test
invitations, disable harmless workflows, and have the setup owner remove
remaining disposable identities/communities/data with their supported tools.
Record cleanup confirmation and any durable revocations that intentionally
remain until fixture deletion. Never reuse these identities for real work.

A passing staging run requires exact deployed-version evidence, every required
case passing, and confirmed cleanup. Failed or skipped cases need concrete
follow-ups and an owner. Identify remaining hosted-fleet revisions, legacy-v1
drain/cutoff gaps, topology gaps, and human testing separately. Staging alone
does not prove every hosted relay is updated or authorize security-finding closure.
