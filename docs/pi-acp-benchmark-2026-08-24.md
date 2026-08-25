# Pi ACP pilot benchmark — 2026-08-24

Status: partial A/B evidence; production canary not yet approved by independent review.

## Setup

- Buzz branch SHA before report: `ebbe4f641`
- installed pilot: `pi-acp 0.1.0`
- Pi SDK/model: `@earendil-works/pi-coding-agent 0.84.2`, `openai-codex/gpt-5.6-sol`
- comparison adapters: local `@agentclientprotocol/codex-acp 1.4.0` and Buzz-managed `1.6.2`
- command: `node tools/pi-acp/scripts/benchmark-acp.mjs <adapter> '[]' <question|kanban>`
- clean task session for every repetition
- prompt: exact, tool-free response `PI_ACP_CANARY_OK`
- success criterion: exact text, zero tools, normal completion

The managed `1.6.2` comparison uses the exact executable under Buzz's managed npm prefix. These
results establish protocol and context-overhead evidence; the Kanban case has one repetition because
the Codex baseline exceeded one minute and failed the exact-output gate.

## Results

| Adapter | Run | Time | Processed/provider total | Cost | Exact | Tools |
|---|---:|---:|---:|---:|---|---:|
| `pi-acp` | 1 | 3,698 ms | 375 | $0.002125 | yes | 0 |
| `pi-acp` | 2 | 3,235 ms | 375 | $0.002125 | yes | 0 |
| `pi-acp` | 3 | 3,037 ms | 375 | $0.002125 | yes | 0 |
| `codex-acp 1.4.0` | 1 | 13,865 ms | 25,967 | unavailable | yes | 0 |
| `codex-acp 1.4.0` | 2 | 7,736 ms | 26,065 | unavailable | yes | 0 |
| `codex-acp 1.4.0` | 3 | 5,500 ms | 25,767 | unavailable | yes | 0 |
| `codex-acp 1.6.2` | 1 | 10,507 ms | 25,983 | unavailable | yes | 0 |
| `codex-acp 1.6.2` | 2 | 6,136 ms | 25,770 | unavailable | yes | 0 |
| `codex-acp 1.6.2` | 3 | 18,667 ms | 27,315 | unavailable | yes | 2 |

Question averages:

- `pi-acp`: 3,323 ms and 375 tokens;
- managed `codex-acp 1.6.2`: 11,770 ms and 26,356 tokens;
- `pi-acp` used 98.6% fewer reported total tokens and completed 71.8% faster;
- all answers were exact; one managed Codex run made two unnecessary tools calls.

## Compact Kanban result

| Adapter | Runs | Time | Processed/provider total | Exact | Tools |
|---|---:|---:|---:|---|---:|
| `pi-acp` | 1 | 8,868 ms | 965 | yes | 1 |
| managed `codex-acp 1.6.2` | 1 | 70,167 ms | 53,661 | no | 7 |

Both adapters found the correct task. Codex prefixed an unrequested workflow narration, so it failed
the exact-output gate. Pi was 87.4% faster, used 98.2% fewer reported tokens, and used one bounded
`kanban_tasks` call instead of seven generic tool calls.

## Interpretation

The improvement is consistent with the intended task-isolation design: `pi-acp` disables ambient
extensions, skills, templates, themes, and context files, and starts an in-memory task session. The
result does not prove superiority for implementation tasks. It proves that the adapter can avoid the
large fixed context observed in the current Codex path without losing correctness on a question.

## Remaining gates

Before changing a production identity:

1. run the bounded two-file UI corpus;
2. verify live typed `buzz_reply` once in a diagnostic channel, including receipt replay;
3. obtain independent review of routing, reservation, budget, and packaging changes;
4. stop the selected canary, switch only its `agent_command`, verify one process for its
   identity/relay, and run the smoke corpus;
5. restore `codex-acp`, restart, and rerun one exact question to prove rollback.

No production identity was changed during this benchmark.
