# User-deletion fencing: message integration

This is the first serving integration after fence storage (PR #8632). It does not enable the real user-deletion worker. Do not install production fences until all required serving paths, fencing inventory, and external-work draining are integrated and deployed on every writer.

## Coverage

| Operation | This change |
| --- | --- |
| New stream messages and replies, kinds 9 and 40002 | Guarded inside event persistence |
| New forum posts and comments, kinds 45001 and 45003 | Guarded inside event persistence |
| HTTP `/events` and WebSocket `EVENT` for these kinds | Same guard and fixed rejection |
| Agent's first owner assignment through `set_agent_owner` | Coordinates with message locks; refuses restricted owners and identities |
| Other kinds, including edits, reactions, profiles, commands, ephemeral typing, and gift wraps | Follow-up work |
| Uploads, git operations, agent execution, and already-started external work | Follow-up work |

The guard is called by both event insertion implementations in `buzz-db/src/store/event.rs`. It runs before the event INSERT, thread metadata, counters, mentions, and post-commit publication. It only applies to the four kinds above. Shared locks remain held until the caller's transaction commits or rolls back. There is no restriction cache and no separate transaction holding the message's identity locks.

## A normal message

1. The transports authenticate and validate the event through their existing paths. The shared ingest path requires the event signer to equal the authenticated identity for these message kinds.
2. The database writer starts its existing transaction and acquires the shared community deletion lock.
3. Read `users.agent_owner_pubkey` for the signer in this community. Do not trust an owner in a session cache or event tag.
4. Acquire shared identity locks in the existing canonical order: the global signer key, the verified owner's global key when present, and the signer's scoped community key.
5. Read ownership again. If it changed while waiting, reject with a retry message; release the transaction instead of acquiring more locks out of order.
6. Read the signer's global and community-scoped restriction rows. Neither lookup reads the owner's restriction. Refuse with `restricted: identity is restricted for user deletion` if either exists. A missing profile does not remove a restriction.
7. Persist the message and its transactional derived records, then commit and release all locks. Continue the existing post-commit publication.

A global signer lock and lookup are retained even for agents because role/profile rows are mutable. The scoped signer lookup is retained even without an ownership row because permanent restrictions must survive profile deletion. This is two exact indexed lookups, not a new batch-read API. The guard reuses the internal locked-read helper after acquiring the full lock set once.

## Ownership assignment

`set_agent_owner_with_operation` remains the first-owner-wins conditional update. Before that update, its existing transaction acquires shared locks for the prospective owner and global agent key, then the agent's scoped exclusive lock. It checks the prospective owner's global restriction and the agent's own restrictions. The exclusive agent lock prevents assignment from changing ownership while a message uses the old relationship. This covers both existing public setter entry points, including authorization callers; it is not a claim that every agent lifecycle operation is now fenced.

## Fencing and in-flight work

The future fencing worker must lock the request, take the user's exclusive identity lock, discover owned agents under that lock, acquire their exclusive scoped locks, and commit all restrictions and the checkpoint together. If a message owns a shared lock first, the worker waits until that message transaction ends. If the worker owns its exclusive lock first, the message waits and then sees the committed restriction.

This transaction only protects message persistence. It does not drain uploads or promise that all post-commit delivery has finished when a fence commits. Those are separate later stages.

## Validation

Database regressions cover both insertion APIs, all four message kinds, human restrictions across communities, agent restrictions within one community, removal/recreation of profiles, both fencing race directions, owner changes while a sender waits, and assignment blocked by an active message or restricted owner. They observe PostgreSQL lock waiters before releasing a lock rather than depending on a sleep to manufacture the race.

The relay regression starts a local listening relay, sends HTTP requests over TCP, and exercises the WebSocket EVENT handler, checks the fixed rejection, verifies no rejected event exists, and confirms rejected replies leave thread counters unchanged. The peer remains able to send after the requested user and agent are fenced.
