# buzz-dataset

Harbor tasks that score **Buzz product behavior**, not just task correctness.
Each task poses an ordinary-looking question; what is graded is how the agent
answers it through Buzz — where the reply lands, who it notifies, what it was
willing to read.

| Task | Layer | Behavior under test |
| --- | --- | --- |
| [`reply-to-thread`](reply-to-thread) | Regression | Answers in the user's thread instead of as a new top-level message |
| [`user-mention`](user-mention) | Regression | Hands the turn back with an event-level `p`-tag mention of the requesting human |
| [`read-named-path-outside-workspace`](read-named-path-outside-workspace) | Regression | Reads a path the user named explicitly instead of refusing it as out of bounds |
| [`create-channel-invite-users`](create-channel-invite-users) | Workflow | Creates a channel with the exact shape, TTL, and membership asked for |
| [`multiline-message`](multiline-message) | Regression | Preserves real newlines and blank-line structure through the CLI publish path |
| [`narrative-agent-names`](narrative-agent-names) | Regression | Names agents in narrative without waking them through `p` tags |
| [`interleaved-agent-reports`](interleaved-agent-reports) | Workflow | Retains and synthesizes every report in a batch of agent messages |
| [`cross-thread-requests`](cross-thread-requests) | Workflow | Keeps simultaneous top-level requests isolated and replies to both exact threads |
| [`ambiguous-user-mention`](ambiguous-user-mention) | Workflow | Resolves duplicate display names and notifies only the intended pubkey |
| [`memory-retrieval`](memory-retrieval) | Regression | Answers from harness-seeded cold memory without the value appearing in channel history |

For `reply-to-thread` and `user-mention` the graded behavior is **deliberately
absent from `instruction.md`** — it has to come from `buzz-acp`'s production
base prompt. Read a task's own `README.md` before editing its instruction or
verifier.

## Evaluation layers

Every task declares `metadata.evaluation_layer` in `task.toml`:

| Layer | Question | Default trials | Typical cadence |
| --- | --- | ---: | --- |
| Regression | Did Buzz preserve a known product contract? | k=1 | Targeted PR, nightly, or pre-release |
| Workflow | How capable is the agent at realistic Buzz work? | k=3 | Nightly or weekly on a fixed condition |

Report regression results per behavior, not as an average capability score.
Use workflow pass rates and trends as the benchmark headline.

Fast verifier fixtures remain ordinary CI. They validate grading logic, but do
not replace agent trials across the model, base prompt, CLI, and relay.

The task identity remains `buzz-native/<task>` in both layers; the wrapper reads
the metadata instead of encoding the layer in task names.

## Running

These tasks need the [`harbor-buzz-orchestra`](../harbor-buzz-orchestra)
harness, which launches the real `buzz-acp` → `buzz-agent` → `buzz-dev-mcp`
stack inside the task container and exports the relay snapshot each verifier
grades. Plain `harbor run` against this directory will not work, and neither
will `harbor run -a oracle` (no `solution/solve.sh` is shipped — the Oracle
agent replaces the Buzz agent, so no relay trial is provisioned).

From the repo root:

```bash
just benchmark \
  --path benchmarks/buzz-dataset/reply-to-thread \
  --manifest benchmarks/harbor-buzz-orchestra/manifests/buzz-native-solo-luna.yaml \
  --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/openai-live.json \
  --n-concurrent 1
```

The task's regression metadata supplies its default `--attempts 1`. Select a
whole layer with `--path benchmarks/buzz-dataset --layer regression` or
`--layer workflow`. If the dataset root is passed without `--layer` or
`--attempts`, the wrapper runs two Harbor jobs so regression gets k=1 and
workflow gets k=3. An explicit `--attempts`/`-k` overrides these defaults and
runs the selected tasks in one job.

The default condition is one solo agent on `gpt-5.6-luna` at
`thinking_effort: medium`, which needs `OPENAI_COMPAT_API_KEY`; see
[the harness README](../harbor-buzz-orchestra/README.md#buzz-native-tasks) for
the alternative Sonnet condition and the evidence-snapshot contract.

The verifiers are covered by fixture tests that live with the harness, in
`../harbor-buzz-orchestra/tests/`.
