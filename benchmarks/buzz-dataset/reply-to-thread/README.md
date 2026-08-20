# reply-to-thread

## What the agent does

Answers a six-month financial projection posted by the trial user
([instruction.md](instruction.md)). The arithmetic is incidental — this task
measures **where** the answer lands: in the user's thread, not as a new
top-level channel message.

> **The instruction deliberately says nothing about threading.** Threading is
> the behavior under test, and it must come from `buzz-acp`'s production base
> prompt rather than from the task prompt. Do not "fix" the instruction by
> telling the agent to reply in-thread — that would make the task measure
> instruction-following instead of product behavior.

## Environment

`python:3.12-slim-bookworm`, no extra packages: the agent never runs in this
container's shell. `BuzzOrchestraAgent` launches the real `buzz-acp` /
`buzz-agent` stack against a dedicated relay, and the agent works entirely
through Buzz. Agent timeout 300s; the manifest's `trial_budget` is the
effective clock.

## Verifier

Reads the post-agent `/logs/artifacts/buzz-evidence.json` snapshot (written by
`BuzzContainerRuntime._collect_evidence` after the agent stops, so the agent
cannot influence it). Every dimension is programmatic; `reward` is the
conjunction of all of them.

| Dimension | Type | Measures |
| --- | --- | --- |
| `evidence_complete` | programmatic | Snapshot is v1, untruncated, has one orchestrator, and resolves the task event and a candidate reply. Harness health, not agent skill — a 0 here means investigate the run |
| `expected_author` | programmatic | The scored message was published by the orchestrator |
| `same_channel` | programmatic | Reply carries the trial channel's `h` tag |
| `reply_to_thread` | programmatic | Reply carries `["e", <task event>, "", "reply"]` — the behavior under test |
| `answer_correct` | programmatic | Month-6 revenue (160,811), month-6 expenses (84,462), and cumulative profit (374,470), each ±1 and each on a line naming it |

`answer_correct` requires the label and the value on the same line so a
work-showing table with a wrong stated answer cannot pass on its intermediate
rows. `instruction.md` asks for that formatting explicitly.

## Layout

```
reply-to-thread/
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
  --path benchmarks/buzz-dataset/reply-to-thread \
  --attempts 1 \
  --manifest benchmarks/harbor-buzz-orchestra/manifests/buzz-native-solo-luna.yaml \
  --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/openai-live.json \
  --n-concurrent 1
```

`harbor run -a oracle` does **not** work here, and no `solution/solve.sh` is
shipped: the Oracle agent replaces `BuzzOrchestraAgent`, so no relay trial is
provisioned and no evidence snapshot is exported. The verifier is covered
instead by positive and negative fixture tests in
`../harbor-buzz-orchestra/tests/test_reply_to_thread_verifier.py` (run from
`benchmarks/harbor-buzz-orchestra`: `uv run --extra dev pytest -q`).
