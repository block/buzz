# Pi ACP pilot benchmark — 2026-08-24

Status: partial A/B evidence; production canary not yet approved by independent review.

## Setup

- Buzz branch SHA before report: `ebbe4f641`
- installed pilot: `pi-acp 0.1.0`
- Pi SDK/model: `@earendil-works/pi-coding-agent 0.84.2`, `openai-codex/gpt-5.6-sol`
- comparison adapter: `@agentclientprotocol/codex-acp 1.4.0`
- command: `node tools/pi-acp/scripts/benchmark-acp.mjs <adapter>`
- clean task session for every repetition
- prompt: exact, tool-free response `PI_ACP_CANARY_OK`
- success criterion: exact text, zero tools, normal completion

The comparison adapter is older than the separately observed managed Codex adapter version `1.6.2`.
These results establish protocol and context-overhead evidence, not a definitive current-version fleet
comparison.

## Results

| Adapter | Run | Time | Processed/provider total | Cost | Exact | Tools |
|---|---:|---:|---:|---:|---|---:|
| `pi-acp` | 1 | 3,698 ms | 375 | $0.002125 | yes | 0 |
| `pi-acp` | 2 | 3,235 ms | 375 | $0.002125 | yes | 0 |
| `pi-acp` | 3 | 3,037 ms | 375 | $0.002125 | yes | 0 |
| `codex-acp 1.4.0` | 1 | 13,865 ms | 25,967 | unavailable | yes | 0 |
| `codex-acp 1.4.0` | 2 | 7,736 ms | 26,065 | unavailable | yes | 0 |
| `codex-acp 1.4.0` | 3 | 5,500 ms | 25,767 | unavailable | yes | 0 |

Averages:

- `pi-acp`: 3,323 ms and 375 tokens;
- `codex-acp 1.4.0`: 9,034 ms and 25,933 tokens;
- `pi-acp` used 98.6% fewer reported total tokens and completed 63.2% faster in this narrow corpus;
- correctness and tool count were equal.

## Interpretation

The improvement is consistent with the intended task-isolation design: `pi-acp` disables ambient
extensions, skills, templates, themes, and context files, and starts an in-memory task session. The
result does not prove superiority for implementation tasks. It proves that the adapter can avoid the
large fixed context observed in the current Codex path without losing correctness on a question.

## Remaining gates

Before changing a production identity:

1. rerun against the exact managed `codex-acp 1.6.2` executable;
2. run compact Kanban intake and bounded two-file UI corpus;
3. verify live typed `buzz_reply` once in a diagnostic channel, including receipt replay;
4. obtain independent review of routing, reservation, budget, and packaging changes;
5. stop the selected canary, switch only its `agent_command`, verify one process for its
   identity/relay, and run the smoke corpus;
6. restore `codex-acp`, restart, and rerun one exact question to prove rollback.

No production identity was changed during this benchmark.
