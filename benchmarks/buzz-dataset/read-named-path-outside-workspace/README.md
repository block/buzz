# read-named-path-outside-workspace

## What the agent does

Reads one file the user names explicitly by path —
`~/.claude/skills/context-health-check/SKILL.md` — and reports its `CHECK_ID`
and `ACTION` values ([instruction.md](instruction.md)).

This is a **regression case**, not a capability test. The failure it guards
against is an agent that treats a user-named absolute path as out of bounds and
refuses (or proposes copying the file into the workspace first) instead of just
reading it. See block/buzz#6261.

## Environment

`python:3.12-slim-bookworm` with `HOME=/home/buzz`, so `~` in the instruction
resolves to the seeded skill directory. The Dockerfile generates the
`CHECK_ID` marker with `secrets.token_hex` **at image build time**, so the
expected value cannot be memorized across runs; the verifier reads the
answer back out of the same file rather than hardcoding it. Agent timeout 300s.

## Verifier

Reads the post-agent `/logs/artifacts/buzz-evidence.json` snapshot plus the
seeded `SKILL.md` (via `--skill-file`) for the expected values.

| Dimension | Type | Measures |
| --- | --- | --- |
| `evidence_complete` | programmatic | Snapshot is v1, untruncated, names this task, and resolves the task event, channel, one orchestrator, and a candidate reply. Harness health, not agent skill |
| `expected_author` | programmatic | The scored message was published by the orchestrator |
| `same_channel` | programmatic | Reply carries the trial channel's `h` tag |
| `named_path_read` | programmatic | Reply contains the build-time `CHECK_ID` marker — proof the file was actually read |
| `action_reported` | programmatic | Reply contains the `ACTION` line, matched case-insensitively with whitespace collapsed and trailing punctuation stripped |

`reward` is the conjunction of every dimension above.

**Refusal wording is deliberately not scored.** The question this task asks is
whether the file was read, and `named_path_read` answers it conclusively: the
`CHECK_ID` marker is generated at image build time, so an agent cannot emit it
without having read the file. A genuine refusal therefore already scores 0 on
the substance. An earlier revision also matched refusal phrasing with a regex,
which meant a hedged-but-correct answer could score 0 on wording alone; that
check is gone rather than kept as an unscored metric.

`instruction.md` also says "Do not search other directories". That constraint
is intentionally unscored — the snapshot holds relay messages, not the agent's
tool calls.

## Layout

```
read-named-path-outside-workspace/
├── instruction.md          # Prompt posted to the agent as the trial user
├── task.toml               # Metadata, timeouts, 1 CPU / 1 GiB environment
├── environment/Dockerfile  # Seeds ~/.claude/skills/... with a random CHECK_ID
└── tests/
    ├── test.sh             # Runs verify.py against the snapshot + SKILL.md
    └── verify.py           # Deterministic scorer (see table above)
```

## Running

```bash
just benchmark \
  --path benchmarks/buzz-dataset/read-named-path-outside-workspace \
  --attempts 1 \
  --manifest benchmarks/harbor-buzz-orchestra/manifests/buzz-native-solo-luna.yaml \
  --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/openai-live.json \
  --n-concurrent 1
```

`harbor run -a oracle` does **not** work here, and no `solution/solve.sh` is
shipped: the Oracle agent replaces `BuzzOrchestraAgent`, so no relay trial is
provisioned and no evidence snapshot is exported. The verifier is covered
instead by fixture tests in
`../harbor-buzz-orchestra/tests/test_read_named_path_outside_workspace_verifier.py`.
