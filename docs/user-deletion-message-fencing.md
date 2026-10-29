# User-deletion fencing: serving integration

This extends the restriction storage from PR #8632 to serving writes. It does not enable the real user-deletion worker. Do not install production fences until inventory, quiescing, worker integration and every writer deployment are complete.

## Identity contract

Human restrictions apply to a public key across hosted communities. Agent restrictions apply to a public key within one community. Serving code checks the signer's global and scoped restrictions. It locks the stored owner's identity for coordination; the owner's restriction is not an additional authorization check for an independently admitted agent.

Restrictions survive ordinary profile deletion. An independently admitted agent with no stored owner or own restriction remains allowed, even if it supplies an optional owner credential. When HTTP admission depends on delegation, required owner linking must succeed before ingestion.

## Coverage

| Surface | Protection |
| --- | --- |
| Persistent events, including replies, reactions, edits, channel changes and deletion commands | Shared identity locks in persistence, plus an actor lease around ingestion and its side effects |
| Replaceable profiles, emoji collections, bookmarks and workflow definitions | Guarded replacement functions and ingestion actor lease |
| Personal read-state mutations | Identity guard in the write transaction, including idempotent retries |
| WebSocket ephemeral activity and observer frames; HTTP typing | Actor lease through publication and final verification |
| Uploads on both routes | Actor lease around the upload, including final verification |
| Git publication and default-branch mutation | Actor lease around the protected publication/mutation |
| Workflow actions | Durable workflow owner lookup and actor lease before each action |
| Invitation mint/claim and GIF share | Actor lease around the authenticated mutation |
| Huddle audio | One actor lease per session; lease loss cancels the connection |

Ordinary reads remain available. Affected-user checks, such as Other User adding Requested User to a channel, are separate work. This is not a claim that all background jobs or external agent processes are cancelled.

## Transactional writes

1. Acquire the existing shared community deletion guard.
2. Read the signer's stored owner in this community.
3. Acquire shared global signer/owner locks in canonical order, then the scoped signer lock.
4. Read ownership again. If it changed while waiting, refuse the operation for retry rather than extending the lock set out of order.
5. Read the signer's global and scoped restriction rows. Refuse a restricted signer.
6. Write the event and transactional derived records; commit or rollback releases the locks.

The restriction reads are ordinary indexed reads. No restriction cache is used. First-owner assignment coordinates through the scoped exclusive agent lock and refuses restricted prospective owners and agents. Both the ordinary setter and invitation claims use this guard; invitation paths acquire their existing admission lock first. Refused claims roll back membership, owner rows and invitation usage together.

## Operations spanning transactions or external effects

The existing `community_serving_write_leases` table gains nullable `actor_pubkey`. Actor lease acquisition checks restrictions while holding shared identity locks, then records the lease in that same transaction. Heartbeats and final verification also check restrictions. No database connection is retained for the entire external operation.

Fence insertion validates the approved request and worker lease, acquires exclusive identity locks, then checks for live actor leases matching the restriction's scope. If one exists, insertion returns `identity writes are still in flight; retry fencing`. The caller must roll back the transaction and retry later; it must not sleep while holding the exclusive locks.

This handles both orders: an admitted operation leaves a lease that prevents fencing, or fencing commits first and the new operation is rejected. It reuses the existing lease heartbeat, expiry and cancellation mechanisms. It does not provide a remote storage service with transactional rollback or stop a remote effect already dispatched before lease loss.

A live audio session can keep its lease alive. The future deletion workflow must quiesce/disconnect sessions before fencing can complete. This PR does not add that orchestration. A worker installing human and owned-agent restrictions must still do so atomically under the complete ordered lock set and save its checkpoint in the same transaction.

## Deployment and performance

Apply migration `0075_serving_write_actor.sql` before deploying code that reads/writes the new column. Desired-schema installations obtain the column and partial actor index from the table definition. Old community-only writers can leave the column NULL, but cannot protect user deletion; deploy the actor-aware code to every writer before enabling real fencing.

The actor lease adds database writes and checks to serving operations, including ephemeral events. No production throughput or latency benchmark is claimed. Schema rollout and serving-path overhead need review.

## Validation

The local relay simulation uses synthetic identities across three communities and real HTTP/WebSocket requests, PostgreSQL, Redis and MinIO. The fresh matrix ran 510 supported cases before fencing and 510 afterward: 272 expected refusals and 238 allowed controls after fencing. Fifteen HTTP presence observations per phase are unsupported and excluded. These are action/actor/community/transport combinations, not 510 distinct features.

The matrix covers reactions, edits, votes, canvas, channel settings/TTL/membership/creation, artifacts, profiles, emoji lists/sets, bookmarks, workflow creation, typing, WebSocket presence, both upload routes and read-state. It verifies persistence for event writes. The same agent remains allowed in a community where it belongs to a different owner.

Separate production-path regressions cover messages and reply counters, conditional canvas, message/workflow/channel deletion with unchanged database snapshots on refusal, existing workflow actions, HTTP delegated ownership, actor lease races, and audio disconnect/fence/new-join. Git tests exercise finalization with real repositories and local MinIO, including publication records and unchanged storage on refusal; they are not a complete remote CLI push test.

Reproduce the focused PostgreSQL checks with isolated test PostgreSQL and Redis configured:

```sh
scripts/postgres-test-run.sh -p buzz-db -p buzz-deletion -p buzz-relay --lib \
  -E 'test(/user_deletion/)' --test-threads 2 --no-fail-fast
```

The live experiment uses approved synthetic requests and production fence helpers. It does not execute a real deletion inventory/workflow, delete user data, or validate production authentication/deployment at fleet scale. Repository-wide validation status is recorded in the PR; a passing live matrix alone is not a passing full suite.
