# create-channel-invite-users

## What the agent does

Creates a temporary private stream channel named `fix-pr-1234` with a one-hour
lifetime and invites an exact subset of a seeded directory: three named users as
members and two named bots with the `bot` role
([instruction.md](instruction.md)).

Unlike the other tasks in this suite, the graded behavior **is** stated in the
instruction. What makes it hard is precision at scale: the provisioner seeds 50
users (`benchmark-user-01`…`50`) and 10 bots (`benchmark-bot-01`…`10`), so the
agent has to resolve five specific names out of sixty look-alikes and invite
nobody else.

## Environment

`python:3.12-slim-bookworm`, no extra packages: the agent never runs in this
container's shell. `BuzzOrchestraAgent` launches the real `buzz-acp` /
`buzz-agent` stack against a dedicated relay, and the agent does all its work
through `buzz channels create` / `channels invite`. Agent timeout 300s.

Directory identities are derived deterministically from the owner key
(`BuzzTrialProvisioner._stable_credential`) without persisting any secret, and
`_seed_directory` skips profiles already published — so reruns are idempotent
and pubkeys are stable across trials.

## Verifier

Reads the post-agent `/logs/artifacts/buzz-evidence.json` snapshot. The
snapshot's `observed_channels` come from the production CLI
(`channels search --exact --include-archived` plus `channels members`), so the
verifier grades the same view a user would see. Every dimension is
programmatic; `reward` is the conjunction of all of them.

| Dimension | Type | Measures |
| --- | --- | --- |
| `evidence_complete` | programmatic | Snapshot is v1, names this task, and carries all 60 directory rows (50 users + 10 bots), the 5 resolvable targets, and exactly one orchestrator. Harness health, not agent skill — a 0 here means the provisioner or relay is suspect |
| `channel_created` | programmatic | Exactly one channel named `fix-pr-1234` exists |
| `channel_shape` | programmatic | `channel_type = stream`, `visibility = private`, not archived |
| `temporary_channel` | programmatic | `ttl_seconds == 3600` — "for one hour", read from the kind:39000 `ttl` tag surfaced by `channels search` |
| `exact_membership` | programmatic | Member pubkeys are exactly the owner plus the 5 targets — no extras, no duplicates |
| `expected_roles` | programmatic | The 3 users hold `member`, the 2 bots hold `bot`, the creator holds `owner` |

## Layout

```
create-channel-invite-users/
├── instruction.md          # Prompt posted to the agent as the trial user
├── task.toml               # Metadata, timeouts, 1 CPU / 1 GiB environment
├── environment/Dockerfile  # Bare python image; the relay stack is uploaded
└── tests/
    ├── test.sh             # Runs verify.py against the evidence snapshot
    └── verify.py           # Deterministic scorer (see table above)
```

To change the target set, edit `task_fixtures.TARGET_USERS` / `TARGET_BOTS`,
`instruction.md`, and the matching constants at the top of `tests/verify.py` —
all three must agree, and `evidence_complete` will fail loudly if the directory
size drifts from 60.

## Running

```bash
just benchmark \
  --path benchmarks/buzz-dataset/create-channel-invite-users \
  --attempts 1 \
  --manifest benchmarks/harbor-buzz-orchestra/manifests/buzz-native-solo-luna.yaml \
  --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/openai-live.json \
  --n-concurrent 1
```

`harbor run -a oracle` does **not** work here, and no `solution/solve.sh` is
shipped: the Oracle agent replaces `BuzzOrchestraAgent`, so no relay trial is
provisioned and no evidence snapshot is exported. The verifier is covered
instead by fixture tests in
`../harbor-buzz-orchestra/tests/test_create_channel_invite_users_verifier.py`.
