# Standalone agent database cleanup

`buzz_deletion::user::agents::handle` executes one bounded operation for a
`purge_agents_db` stage in one approved community. The main worker, transition
rules and CLI do not call it yet. No new tables or migrations are required.

One stage handles all agents in that community. Agent progress lives in
`entity_deletion_stage_history.metadata_json.progress`; the request row still
owns the worker lease, generation, heartbeat and retry deadline. Agents do not
have separate leases or workflow stage rows.

## Approved scope and prerequisites

Real inventory generation includes `agents: [{"pubkey": "<hex>"}]` on each
discovered community, using `users.agent_owner_pubkey` in the same SQL snapshot
as community discovery. The digest and approval cover this exact agent list.
Limits are 1,024 agents per community and 4,096 per request; exceeding either
fails rather than returning partial scope. Keys must be canonical and unique.

Existing inventories remain unchanged and readable. An absent `agents` field
does not authorize agent deletion; `agents: []` explicitly authorizes empty
scope. Demo inventories/execution are unchanged. Discovery still uses current
relay/channel memberships: this change does not discover every historical
community or enable owned-community execution. There is no command to refresh
and reapprove an existing saved inventory in this PR.

The caller supplies the approved real-user request, current stage, live lease,
trusted relay signing key and batch size (1–100). It must enforce gating and
drain affected writers/workflows/deliveries before calling. Fence storage alone
does not enforce serving paths. The handler requires this request's global
owner fence and each agent's community fence with matching owner provenance.

Mismatched live ownership, newly owned agents outside the approved list, invalid
saved progress and the relay signing identity are refused. A missing profile
does not erase approved scope: recovery uses saved inventory and fence evidence.
Scope changes require reconciliation, not silently dropping or adopting agents.

## One handler call

1. Reload/start the current stage through the existing stage API; check the
   request lease, approval and retry deadline.
2. Take the community and request locks. Validate exact saved inputs and
   fences, and reject active external-write leases.
3. Read progress and choose the first unfinished agent in inventory order.
4. Perform one bounded operation in that agent's current phase. When exhausted,
   the policy in `buzz-deletion` selects the next internal phase. SQL effects
   and the progress update commit in the same transaction.
5. After final verification, mark that agent completed with a database-clock
   timestamp. A subsequent call starts the next agent.
6. When every agent is complete, save the stage result through the existing
   completion API. Only the workflow transition function advances the stage.

The result is `AgentsDbBatch { progress, exhausted }`. The caller repeats while
not exhausted, heartbeats between calls and owns lease release/transitions.
There is no new executor or unbounded loop inside the handler.

| Internal phase | Work |
| --- | --- |
| `messages` | Shared user message cleanup: authored/trusted-relay-attributed messages, dependent reactions/edits/votes, delivery records, terminal workflow copies and thread-count repair. |
| `private_state` | Authored lists/settings (3, 10000–10003, 30000, 30003, 30315, 30078), engrams (30174), metrics (44200), private reminders (30300), personal read frontiers/accounts. |
| `job_records` | Agent-authored job protocol events 43001–43006. Other participants' signed events remain. |
| `owned_workflows` | Owned workflow definitions, scheduled fires, approvals and terminal runs, then authored definition events (30620). Active runs block deletion. |
| `delivery_state` | Agent push outbox/leases, listener outbox where the agent is sender/recipient, owned subscriptions/delivery logs, authored lease events (30350). Live delivery claims block deletion. |
| `verify_db_content` | Read-only absence checks; remaining supported data prevents completion. |

Non-message events reuse the same dependent-record, media and thread-repair
checks. Workflow/subscription children are removed before their parent, leaving
the parent as a recovery anchor between batches. Owned-workflow cleanup removes
scheduled-fire claims; removing message copies from somebody else's workflow
preserves that workflow and its scheduled-fire claim.

## Progress and retry

Illustrative progress (real keys have 64 hex characters):

```json
{
  "version": 1,
  "current_agent": "<agent-b>",
  "agents": {
    "<agent-a>": {
      "status": "completed",
      "phase": "verify_db_content",
      "completed_at": "2026-10-30T20:00:00Z",
      "last_error": null
    },
    "<agent-b>": {
      "status": "failed",
      "phase": "owned_workflows",
      "completed_at": null,
      "last_error": "agent-owned workflow runs must stop before deletion"
    }
  }
}
```

`inputs`, predecessor information and other stage metadata are preserved.
`current_agent` can be null between completed agents; approved order determines
the next unfinished entry. Completed agents form a prefix and are skipped.

- Batch error: roll back data and progress together. A separate transaction
  marks the current agent/stage failed and the request `retry_wait`, with a
  30-second deadline and bounded error. The error still propagates.
- Retry: reload the same stage and failed phase, scanning remaining rows.
  There is no row-offset cursor. Earlier completed agents stay completed.
- Crash after commit: PostgreSQL retains the checkpoint. A replacement worker
  resumes it, including when the same worker ID gets a newer lease generation.
  Stale-generation writes are rejected.
- Lease expiry during a batch: rollback. An expired worker cannot save failure;
  the durable current stage is available for another worker to reclaim.
- Final completion-save failure: all agent checkpoints remain complete. Retry
  only needs to save stage completion, without repeating those agents.
- Invalid inventory/progress or unavailable PostgreSQL may prevent recording
  failure too. Both errors propagate; they never become successful empty scope.

## Retained data and integration limits

Profiles/profile events, owner-authored managed configuration, credentials,
memberships, shared emoji/artifacts, audit/moderation records, global listener
registrations, media and hosted resources remain for other stages. Global
registrations may span communities. Completion here does not erase the identity.

Attachment/link-bearing events still block deletion, including private/job
events. Workflow definitions with links also block removal. The conservative
check includes ordinary URLs. There is no media bypass: preserving references
for later media cleanup remains an integration requirement. A verification
failure from an ungated writer requires fixing that writer and reconciling
progress; retrying verification cannot rerun an earlier finished phase. No
administrative reset is added here.

The 1–100 limit bounds primary rows in each operation; dependent cleanup and
verification scans can be larger. Existing message dependency/thread caps,
two-second lock timeout, ten-second statement timeout and final lease-expiry
check apply. Statement timeouts are not whole-batch deadlines. Heartbeats cannot
renew while the request row is locked. Measure large-account scans and the
transaction lease budget before wiring this into serving workflows.

## Code and tests

- `buzz-deletion/src/user/agents.rs`: phase order, outcomes, final completion.
- `buzz-db/src/store/user_deletion/agents/`: scoped SQL, progress validation,
  atomic checkpoints and failure updates.
- `buzz-db/src/store/user_deletion/execution.rs`: shared admission/lease checks;
  `messages/`: shared message/dependent cleanup.

```sh
scripts/postgres-test-run.sh -p buzz-deletion -p buzz-db --lib \
  -E 'test(user::agents::) | test(user::messages::) | test(user_deletion::)'
```

Tests use real inventories, approvals, fences and leases in isolated databases.
They cover multiple agents, restart, completed-agent preservation, same-owner
reclaim, transactional rollback, expiry at checkpoint, completion-save recovery,
missing profiles, scope/fence/progress refusal, active workflow/delivery blockers
and final verification. Existing message tests cover shared thread/attribution
behavior. There is no operator CLI path for this standalone handler yet.
