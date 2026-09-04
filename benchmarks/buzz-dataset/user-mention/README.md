# user-mention

## What the agent does

Answers a one-line licensing calculation ([instruction.md](instruction.md)).
The arithmetic is incidental — this task measures whether the agent hands the
turn back with an **event-level mention** of the requesting human, so the user
gets a real Buzz notification instead of a message they have to notice.

> **The instruction deliberately says nothing about mentioning anyone.** The
> mention is the behavior under test and must come from `buzz-acp`'s production
> base prompt. Do not add "mention the user" to the instruction.

The trial user for this task is provisioned with the stable three-word display
name `John Vincent Doe` (`task_fixtures.USER_MENTION_DISPLAY_NAME`), which
forces the agent to resolve a multi-word identity to a pubkey rather than
guessing a single-token handle.

## Environment

`python:3.12-slim-bookworm`, no extra packages: the agent never runs in this
container's shell. `BuzzOrchestraAgent` launches the real `buzz-acp` /
`buzz-agent` stack against a dedicated relay. Agent timeout 300s.

## Verifier

Reads the post-agent `/logs/artifacts/buzz-evidence.json` snapshot. Every
dimension is programmatic; `reward` is the conjunction of all of them.

| Dimension | Type | Measures |
| --- | --- | --- |
| `evidence_complete` | programmatic | Snapshot is v1, untruncated, names this task, and resolves exactly one orchestrator, one user, and a candidate reply. Harness health, not agent skill |
| `three_word_user` | programmatic | The provisioner seeded the three-word display name. Fixture self-check |
| `expected_author` | programmatic | The scored message was published by the orchestrator |
| `same_channel` | programmatic | Reply carries the trial channel's `h` tag |
| `user_p_tagged` | programmatic | Reply carries a `p` tag for the user's pubkey — the behavior under test. Presentation-only `@text` does not count |
| `answer_correct` | programmatic | Annual total 5,328 (12 licenses × $37 × 12 months), ±1 |

## Layout

```
user-mention/
├── instruction.md          # Prompt posted to the agent as the trial user
├── task.toml               # Metadata, timeouts, 1 CPU / 1 GiB environment
├── environment/Dockerfile  # Bare python image; the relay stack is uploaded
└── tests/
    ├── test.sh             # Runs verify.py against the evidence snapshot
    └── verify.py           # Deterministic scorer (see table above)
```

## Running

```bash
just benchmark \
  --path benchmarks/buzz-dataset/user-mention \
  --attempts 1 \
  --manifest benchmarks/harbor-buzz-orchestra/manifests/buzz-native-solo-luna.yaml \
  --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/openai-live.json \
  --n-concurrent 1
```

`harbor run -a oracle` does **not** work here, and no `solution/solve.sh` is
shipped: the Oracle agent replaces `BuzzOrchestraAgent`, so no relay trial is
provisioned and no evidence snapshot is exported. The verifier is covered
instead by fixture tests in
`../harbor-buzz-orchestra/tests/test_user_mention_verifier.py`.
