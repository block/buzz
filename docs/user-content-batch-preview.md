# Print or delete human-content batches

This operator command selects one human's supported records in one community.
By default it only prints the inventory in batches and changes no data. Explicit
`--print-only` does the same; `--delete` prints each batch and commits its operations.
The two flags cannot be combined. `--read-only` remains an alias for `--print-only`.
Neither mode starts the durable account-deletion
workflow, runs migrations, or marks an account erased.

```sh
# DATABASE_URL identifies the database. Only public keys are needed.
cargo run -p buzz-admin -- user-deletions preview-batches \
  --pubkey '<npub-or-hex>' --community '<community-uuid>' \
  --print-only > batches.jsonl

# Stop the relay and external workers first. This permanently removes rows.
cargo run -p buzz-admin -- user-deletions preview-batches \
  --pubkey '<npub-or-hex>' --community '<community-uuid>' \
  --delete > deletion.jsonl
```

The community is required even when the key currently belongs to only one;
historical content after leaving is included. For relay-signed message history,
optionally supply `--relay-pubkey '<trusted-relay-public-key>'`. Only that signer
may use the existing actor/p attribution rules. Added personal and command kinds,
notes and articles require direct authorship. No private keys are accepted.

## Output and recovery

Stdout is JSONL; diagnostics and errors use stderr. Each `batch` names its
community, number, exact table keys, proposed actions and selection reasons.
An approval's `token_hash` identifies its stored hash, never a plaintext token.
Event bodies, approval notes and endpoint grants are not printed.

Preview emits `preview_start`, `batch` records and `preview_complete`. It uses a
repeatable-read, read-only transaction and a read-only connection. A SELECT-only
database account supplies an additional boundary. Later live writes are outside
that snapshot.

Deletion emits `delete_start`, then for each batch:

1. Print and flush `batch` before executing any of its operations.
2. Execute the ordered operations and repair affected thread metadata in one
   transaction.
3. Commit, then print `batch_committed` with the batch number and item count.

`delete_complete` appears only after a fresh scan finds no remaining supported
operations. **A printed batch alone is not a commit receipt.** A crash or output
failure after commit can lose the receipt even though the rows are gone. Rerun
the command: it selects remaining database rows, retaining source events and
parents until their dependent records are removed. Earlier committed batches
remain deleted when a later batch fails; there is no whole-run undo.

Zero exit status and the appropriate completion marker prove exhaustion only
for the declared scope. No marker is emitted on query/output failure, cycles,
excessive depth or item limits. A saved preview cannot be imported for execution;
`--delete` always generates its own plan against the current database.

## Combined table ownership

The same planner and executor own these tables. A row matching multiple selection
paths is printed, deleted and counted once.

| Scope | Selection and ordering |
| --- | --- |
| Events | Authored chats, edits, reactions, forum posts/comments/votes, individual pins/bookmarks, schedules/reminders/diffs, notes/articles, personal lists/status/app state/private-state records, job records, workflow definitions, push records, ordinary deletion requests, DM commands and workflow trigger/approval/denial commands. Exact kinds are printed in the header. Stored history and soft-deleted rows are included. Removing command history does not replay or reverse it. |
| Event dependencies | Incoming reactions, edits and votes in the same community/channel. Edits/votes use the first valid `e` tag, reactions the last. Never follow outgoing targets to remove someone else's message. Ordinary replies stay. |
| Event copies | `reactions`, `event_mentions`, `push_match_queue`, `push_wake_outbox`, `operator_listener_outbox`, `delivery_log`, and finished workflow runs tied to the selected event. |
| Workflows | Workflows owned by the human, their scheduled fires, approvals and finished runs, then the workflow parent. Saved trigger-author sweeps also find runs whose source event is gone. Approvals and run pointers precede runs. |
| Surviving schedules | Clear only the pointer to a removed run; retain another owner's scheduled-fire claim so it cannot execute again. Owned workflow schedules are deleted. |
| Subscriptions | Human-owned subscriptions' delivery logs, then subscriptions. Another owner's filter mentioning this human does not establish ownership. |
| Notifications | Push wake-ups selected by recipient `author`, then that human's `push_leases`; listener outbox rows selected by target or receiving listener. Source messages belonging to others stay. |
| Thread metadata | Remove unused structure and recompute affected counts/times. Preserve ancestry needed by surviving replies, without retaining deleted event payloads. |

Read positions and separate access/global tables belong to a different handler.
Profile and agent cleanup remain separate. Shared canvases, artifacts, Git,
huddles, emoji/persona/team definitions, gift-wrap attribution, and human approval
copies on somebody else's surviving workflow remain unresolved coverage. They
are not silently erased or described as completed. The footer counts authored
event kinds outside this handler; it is not a complete inventory of other tables.

## Execution checks and limits

Run deletion with the relay and external workers stopped. This standalone tool
does not cancel jobs, stop processes, revoke credentials or install durable user
fences. It holds the existing exclusive community advisory lock across planning
and commits, plus the shared schema-destruction lock; checks the scoped-table inventory;
and refuses inactive communities or live external serving leases. It owns and
closes its database connection so cancellation cannot leave session locks in the
pool. Compliant community writers wait until it releases the lock. This is not
production account-deletion orchestration.

The entire plan is checked before the first deletion. Active workflow runs or
deliveries, inconsistent approval/run ownership, media references and potentially
encrypted payloads prevent execution. Media evidence integration is not supplied
by this command: a URL scan cannot authorize deleting opaque payloads. Workflows,
run traces, approval notes and delivery errors are checked too. No bypass flag is
provided. Preview can still print blocked records for inspection.

Each row is locked and checked against its preflight fingerprint before mutation.
Parents cannot be removed while children remain; foreign-key cascades are not
used to hide extra deletions. SQL errors roll back the current batch. A final
remaining-row scan precedes the completion marker.

`--batch-size` is 1–100 (default 100), including mixed tables and event types.
Thread repairs are reported separately; their work is not bounded by the item
count. `--max-items` bounds unique planned operations to 1–1,000,000 (default
1,000,000). Deletion retains this bounded plan in memory before execution;
large accounts may require substantial memory. Dependency depth is limited to
64. Each statement has a 30-second timeout and locks a 3-second timeout. These
limits do not bound total run time or community lock duration. Measure large
accounts before operational use.

## Mode-aware handler API

The command passes `ExecutionMode::ReadOnly` or `ExecutionMode::Delete` to
`run_message_batches(options, mode, writer)`. The handler owns the mode branch.
Both modes use the same content planner; delete adds execution guards and
commits the selected batches. Existing preview/delete methods remain compatibility
entry points over that shared planner. New interface regressions are prepared;
validation of this refactor is pending.
