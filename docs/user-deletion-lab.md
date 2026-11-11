# Local user-deletion lab

This lab prepares repeatable content, media and account cleanup scenarios in
isolated local services. It is a development fixture, not a production erasure
tool. The combined setup and workflow journey are **pending validation**.

The operator controls each phase. Preparing this script does not authorize its
execution. In an agent-assisted session, get explicit approval before builds,
setup, preview or deletion when execution is on hold. Setup never invokes a
deletion handler; preview never advances the destructive workflow.

## Prepare the environment

Requirements: Docker Compose, Python 3, PostgreSQL 17 client tools (`psql`),
AWS CLI, and the repository's Hermit tools. Put `psql` and `aws` on PATH, or set
`PSQL` and `AWS` to their executable paths. Run commands from the repository root.
After approval, build the local executables:

```sh
. ./bin/activate-hermit
cargo build -p buzz-admin -p buzz-relay
cargo build -p buzz-test-client --example user_deletion_live_seed
```

The default script action prints a plan. The explicit `setup` action starts
Postgres, Redis and versioned MinIO, applies the desired schema, creates the
fixture, starts a local relay for signed activity and read-back, and stops that
relay before returning:

```sh
python3 scripts/user-deletion-lab.py
python3 scripts/user-deletion-lab.py setup
```

Services use Compose project `buzz-deletion-lab`: Postgres on `57431`, Redis on
`57432`, MinIO on `59010` (console `59011`). The temporary relay uses `58300`,
health `58301`, metrics `58302`. All published service ports bind loopback.
Settings are fixed to these disposable services, ignoring inherited service
configuration. The script never uses real user keys.

The fixture includes:

- Two communities and two synthetic people, with target and survivor membership
  in both communities. Secret scalars 1 and 2 are public test keys.
- 250 target messages, edits, reactions, mentions, survivor replies and an event
  with the same key in another community to detect tenant leakage.
- Exclusive, shared, cross-community, thumbnail, unposted and unrelated uploads,
  including multiple exact S3 versions and uploader/publication records.
- Credentials, invites, removed memberships, policies, archive/replay controls
  and 1,000 read-position rows across both users and communities.
- Real signed channel/membership events, four messages, two reactions and three
  uploads admitted and retrieved through the local relay.

SQL fixtures exercise historical selector/storage cases with synthetic
signatures. The signed helper proves admission and retrieval for its smaller
live scenario. These are separate kinds of evidence; SQL alone does not prove
client rendering or relay admission.

Artifacts live in `.local/user-deletion/`, which is ignored by Git. `lab.json`
records identities and whether seeding completed. Each component saves a
manifest with expected rows or object versions. Logs use unique attempt names.
Existing manifests or populated services cause setup to refuse reuse; partial
setup stays marked incomplete. Inspect failures before recreating a disposable
lab. There is no automatic reset or volume deletion.

## Save independent requests and run the read-only workflow

After setup and approval to proceed:

```sh
python3 scripts/user-deletion-lab.py prepare
```

`prepare` submits and approves two requests for the synthetic target through
the real admin CLI, then saves baselines. `request.json` identifies deletion;
`rehearsal.json` identifies the separately approved print run. Rehearsal intake
uses `submit --pubkey <key> --read-only`. Its opaque target is `rehearsal:<key>`
and its inventory binds `execution_mode: rehearsal`; it cannot be switched to
deletion. A destructive request always starts independently at media.

After approval, invoke the worker directly for each step:

```sh
. .local/user-deletion/env
REHEARSAL_ID=$(python3 -c 'import json; print(json.load(open(".local/user-deletion/rehearsal.json"))["id"])')
RELAY_PUBKEY=f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
target/debug/buzz-admin user-deletions run "$REHEARSAL_ID" \
  --read-only --next-step \
  --media-base http://localhost:58300/media --relay-pubkey "$RELAY_PUBKEY"
target/debug/buzz-admin user-deletions inspect "$REHEARSAL_ID"
```

The ordinary worker claims and heartbeats the request, loads its saved stage,
runs the real handler in print mode, persists its result, transitions to the
next stage, and releases its lease. No script chooses the next handler. Invoke
the same command again only after inspecting the saved result and successor.
The sequence is media, content/account for the first community, then
content/account for the second. The next invocation reports `remaining_cleanup`
with exit code 2 and `account_erasure_complete: false`.

Only the rehearsal's journal changes. Handler database reads use a separate
read-only connection; media uses storage reads. After every step:

```sh
python3 scripts/user-deletion-content-fixture.py verify --manifest .local/user-deletion/content.json --phase preview
python3 scripts/user-deletion-media-fixture.py verify --manifest .local/user-deletion/media.json --phase preview
python3 scripts/user-deletion-account-fixture.py verify --manifest .local/user-deletion/account.json --phase rehearsal --rehearsal-request "$REHEARSAL_ID"
```

The account verifier permits journal progress for only that request, while
checking source data, approvals and every other request against the baseline.
Incomplete media decisions or capped account output fail the current stage
without advancing. Content limit exhaustion also fails. Correct the cause and
resume after the saved retry deadline. A completed current stage is transitioned
on restart without running its handler again.

Rehearsal observations do not simulate the mutations of earlier stages and
are not deletion evidence. The destructive run rescans and establishes its own
media prerequisites. Keep the lab quiet for reproducible proposal comparisons.

### Optional strictly read-only preview

Default `run`, `--read-only` and `--print-only` retain their no-write behavior:
they neither claim a request nor advance it. `user-deletion-lab.py preview`
remains a convenience for those observations, not the worker rehearsal above.
Incomplete media/account observations return exit code 2.
Run that convenience check before starting rehearsal: its strict comparison
includes the original workflow baseline and will correctly flag later journal
changes, even when the preview itself made none.

Before deletion, also check capped account preview (`--stage purge_account
--max-items 1`) returns 2 and does not change baselines. Verify default and
`--print-only` spellings produce the same proposals. These checks are pending
in the combined journey.

## Delete one handler at a time

Deletion needs separate explicit operator approval after inspecting rehearsal.
The setup script has no delete action. Stop the relay and every worker using
this database before invoking `--offline`; this flag is the operator's assertion
that they have stopped. It does not stop processes or disable networking.
The deletion worker still connects to Postgres and object storage. Live
serving-write leases also cause refusal; this point-in-time check is not an
online write fence. Rehearsal does not require `--offline`.

```sh
. .local/user-deletion/env
REQUEST_ID=$(python3 -c 'import json; print(json.load(open(".local/user-deletion/request.json"))["id"])')
RELAY_PUBKEY=f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9

target/debug/buzz-admin user-deletions inspect "$REQUEST_ID"
# Only after approval for this step:
target/debug/buzz-admin user-deletions run "$REQUEST_ID" \
  --delete --offline --next-step \
  --media-base http://localhost:58300/media --relay-pubkey "$RELAY_PUBKEY"
```

Each invocation executes one handler and advances to the next. Inspect its saved
counters and verify the resulting state before proceeding:

| Completed stage | Verification |
| --- | --- |
| Request-wide `purge_media` | Media fixture `verify --phase after-media`; content fixture must still match preview. |
| First community `purge_content` | Content fixture `verify --phase after-content`. |
| First community `purge_account` | Account fixture `verify --phase after-account`. |
| Second community `purge_content` | Content fixture with both repeated `--completed-community` values; media fixture `--phase after-content`. |
| Second community `purge_account` | Account fixture with both repeated `--completed-community` values. |

For each script, pass `--manifest .local/user-deletion/<component>.json`.
Community IDs are in `lab.json`, in execution order. See the
[account fixture](user-deletion-account-fixture.md),
[media fixture](user-deletion-media-fixture.md), and
[live helper](user-deletion-media-fixture.md#signed-live-relay-fixture) documentation for their contracts.
The live helper needs a running relay for HTTP/WebSocket read-back. Restart it
only for an approved read-back phase, then stop it before further deletion.
Do not interpret failure to connect as successful deletion.

After both communities, the workflow stops at `remaining_cleanup` and returns
exit code 2. Verify that the request never reports full erasure. Repeating that
boundary must not remove more source data. Repeat the component verifiers as a
second independent check.

This scope leaves archive/replay, required actor references, moderation/audit,
devices, global records, cache/session invalidation, profiles and owned agents.
A retained target profile may still reference media removed by the media stage.
`account_erasure_complete` stays false. This is a supported-scope cleanup lab,
not evidence that the person's entire account has been erased.

## Handler contract

The worker supplies `ExecutionMode::ReadOnly` or `ExecutionMode::Delete` through
one dispatcher call. Stage routing selects `media_handler::run`, `content::run`
or `account::run`; each receives the same `HandlerContext` containing source DB,
options, request, lease, stage, subject and mode. The handlers own mode decisions
and reuse their existing scanners, selectors and planners. Delete-specific
ownership checks and mutations remain inside the handlers.

For `run`, `--read-only` selects the persisted print workflow (`--rehearse` is
an alias). `--print-only` or omitted mode retains the stateless inspection path.
Both worker modes persist claims, results and transitions; only delete changes
source data. Separate request identities prevent printed stages from counting
as completed deletion. Each read-only stage observes original data, so its
proposal is not a simulation of earlier stages' deletions.
