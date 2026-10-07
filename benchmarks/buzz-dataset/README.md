# buzz-dataset

Harbor tasks that score **Buzz product behavior**, not just task correctness.
Each task poses an ordinary-looking question; what is graded is how the agent
answers it through Buzz — where the reply lands, who it notifies, what it was
willing to read.

| Task | Behavior under test |
| --- | --- |
| [`reply-to-thread`](reply-to-thread) | Answers in the user's thread instead of as a new top-level message |
| [`user-mention`](user-mention) | Hands the turn back with an event-level `p`-tag mention of the requesting human |
| [`read-named-path-outside-workspace`](read-named-path-outside-workspace) | Reads a path the user named explicitly instead of refusing it as out of bounds |
| [`create-channel-invite-users`](create-channel-invite-users) | Creates a channel with the exact shape, TTL, and membership asked for |
| [`multiline-message`](multiline-message) | Preserves real newlines and blank-line structure through the CLI publish path |
| [`narrative-agent-names`](narrative-agent-names) | Names agents in narrative without waking them through `p` tags |
| [`interleaved-agent-reports`](interleaved-agent-reports) | Retains and synthesizes every report in a batch of agent messages |
| [`cross-thread-requests`](cross-thread-requests) | Keeps simultaneous top-level requests isolated and replies to both exact threads |
| [`ambiguous-user-mention`](ambiguous-user-mention) | Resolves duplicate display names and notifies only the intended pubkey |

For `reply-to-thread` and `user-mention` the graded behavior is **deliberately
absent from `instruction.md`** — it has to come from `buzz-acp`'s production
base prompt. Read a task's own `README.md` before editing its instruction or
verifier.

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
  --attempts 1 \
  --manifest benchmarks/harbor-buzz-orchestra/manifests/buzz-native-solo-luna.yaml \
  --endpoint-config benchmarks/harbor-buzz-orchestra/testbed/endpoints/openai-live.json \
  --n-concurrent 1
```

Pass `--path benchmarks/buzz-dataset` to run the whole suite. The default
condition is one solo agent on `gpt-5.6-luna` at `thinking_effort: medium`,
which needs `OPENAI_COMPAT_API_KEY`; see
[the harness README](../harbor-buzz-orchestra/README.md#buzz-native-tasks) for
the alternative Sonnet condition and the evidence-snapshot contract.

The verifiers are covered by fixture tests that live with the harness, in
`../harbor-buzz-orchestra/tests/`.
