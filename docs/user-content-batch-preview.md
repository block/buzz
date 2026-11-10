# Preview human-content cleanup batches

This operator command prints a proposed cleanup sequence for one public key in
one community. It has no deletion executor. It neither creates a deletion request
nor changes workflow progress, counters, rows, media or delivery state.

```sh
# DATABASE_URL must already identify the database to inspect.
cargo run -p buzz-admin -- user-deletions preview-batches \
  --pubkey '<npub-or-hex>' \
  --community '<community-uuid>' \
  --read-only > batches.jsonl
```

Only public keys are accepted. The community is required even when a key currently
belongs to just one community; historical content after leaving is included.
Use a database account with SELECT privileges for an additional boundary.
The command uses a read-only connection and a repeatable-read, read-only
transaction. It does not run migrations or the durable deletion worker.

For historical events signed by a relay on behalf of a person, additionally pass
`--relay-pubkey '<trusted-relay-public-key>'`. Only this signer may use the existing
actor/p attribution rules. Without it, the header explicitly says that relay
attribution is not included. A private key is never needed.

## Read the output

Stdout contains JSON lines; startup diagnostics and errors use stderr:

1. `preview_start`: subject, community, covered kinds, attribution and limits.
2. `batch`: sequential number, community and up to 100 ordered `items`.
3. `preview_complete`: total batches, unique items, warnings and excluded kinds.

Each item names a table, exact key within the batch's community, proposed action,
selection reason and any warning. An approval's `token_hash` identifies its stored
`token` column; no plaintext approval tokens or event contents are printed.
Actions are `would_delete` and `would_clear_workflow_run_id`. A batch can mix tables
and event kinds. `thread_repairs` describes affected events and stored roots whose
metadata would need repair after that batch; it does not expand or simulate those
metadata mutations, and they are not counted in the 100 row operations.

```sh
jq 'select(.record == "batch") | {number, items}' batches.jsonl
jq 'select(.record == "preview_complete")' batches.jsonl
```

A zero exit status **and** `preview_complete` mean the declared preview scope was
exhausted. They do not mean all account data is covered or that deletion is safe
to execute. On a query/output error, cycle, excessive depth or item limit, the
command fails without that marker. Discard partial output; rerun from the start.
No position is saved.

## Selection and batching

Sources are authored events in timestamp/ID order across threads: chat and
historical chat kinds, forum posts/comments, reactions, edits, votes, individual
pins/bookmarks, schedules, reminders, diffs, short notes and article versions.
The exact numeric kinds are printed in the header. Old and soft-deleted stored
versions are included. Notes/articles require direct authorship.

For each source, the iterator follows incoming reactions, edits and votes within
the same community and channel, including a NULL channel. Edits/votes use the
first valid `e` tag and reactions the last. It never follows a source's outgoing
target to delete somebody else's message. Other people's ordinary replies stay.
Dependent cleanup comes before the selected event; workflow approvals and run
pointers come before their run. Direct author sweeps also find remaining reaction
rows and workflow trigger copies with missing source events.

The iterator remembers already-emitted exact keys in memory. It fills each batch
before printing it, including across event boundaries, without changing the
database. For example, 50 reactions with exactly two stored rows each fill one
100-item batch. Additional mention or delivery records consume additional slots.
A message with many dependents spans batches and appears after the last dependent.

`--batch-size` accepts 1–100 (default 100). `--max-items` bounds the in-memory
simulation to at most 1,000,000 unique row operations (also the default).
Exceeding the limit is an error, never a silently truncated success. Dependency
depth is limited to 64. Queries have a 30-second statement and 3-second lock
timeout. This is a diagnostic tool: snapshot duration, dependency lookups and
side-table key sorting still need measurement on large real datasets.

## Scope and prerequisites

This is a content-handler preview, not a whole-account erasure inventory. Shared
canvases/artifacts, command history, profiles, memberships, agents and other
non-message categories are not selected here. The footer counts authored event
kinds outside the handler. Media references, active workflow runs and claimed
delivery work are flagged while the preview continues to print the full proposed
sequence. Those warnings are not permission to execute any of it. Write fences,
media evidence, worker drain, authorization and surviving-thread repair remain
requirements for a future executor.

The snapshot is observational: live writes after it starts are not included.
Compare previews for different users or batch sizes by rerunning the command;
never treat a saved preview as an executable deletion manifest.
