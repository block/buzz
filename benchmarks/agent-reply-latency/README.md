# Agent mention-to-reply latency

`buzz-acp` emits a content-free `mention_reply_latency` observer event after it
has seen both the first self-authored reply on the relay and `turn_completed`.
The event contains one sample plus rolling p50, p95, and max summaries for the
most recent 100 samples on the same warm or cold path.

This first slice measures `harness_relay_receipt` to
`harness_relay_fanout`, recorded explicitly as `measurementStart` and
`measurementEnd`. It does not yet claim sender-publish-to-recipient-render
end-to-end timing.

## Boundaries

All duration math uses `monotonicMs`, a process-local monotonic clock. RFC3339
timestamps and Nostr `created_at` values are correlation metadata only and must
not be used for cross-process duration math.

| Metric | Start | End |
| --- | --- | --- |
| `receiveToQueueMs` | relay frame accepted by `buzz-acp` | event admitted to the channel queue |
| `queueWaitMs` | event admitted to the channel queue | turn task started |
| `sessionResolveMs` | turn task started | ACP session created or reused |
| `postSessionSetupMs` | ACP session created or reused | ACP `session/prompt` written |
| `turnSetupMs` | turn task started | ACP `session/prompt` written |
| `timeToFirstOutputMs` | ACP `session/prompt` written | first semantic ACP model/tool output frame |
| `firstOutputToReplyMs` | first semantic ACP model/tool output frame | first self-authored kind-9 reply observed on the relay |
| `turnDurationMs` | turn task started | turn task completed |
| `totalMs` | relay frame accepted by `buzz-acp` | first self-authored kind-9 reply observed on the relay |

`path` is `cold` when the turn created a new ACP session, `warm` when it reused
one, and `unknown` when session resolution was not observed. Summaries never mix
these paths.

## Deterministic check

The unit fixture drives fixed semantic boundaries through the collector and
asserts stage math, nearest-rank percentiles, warm/cold separation, flat-thread
correlation, trace expiry, and content redaction:

```bash
. ./bin/activate-hermit
cargo test -p buzz-acp latency --no-fail-fast
cargo test -p buzz-acp observer_emits_derived_latency_sample --no-fail-fast
```

## Live benchmark

Managed Desktop agents already start with `BUZZ_ACP_RELAY_OBSERVER=true`. For a
standalone harness, set that variable and make sure the agent has a resolvable
owner so encrypted observer frames can be published. Keep the default
`--ignore-self` behavior enabled; that lets mention-only subscriptions observe
self-authored replies for telemetry without routing them back into the agent.

1. Record the Buzz commit, relay, agent runtime, provider, model, machine, and
   network condition.
2. Open the managed-agent session's raw event rail.
3. Send a fixed prompt such as `Reply with exactly: pong.` and wait for its
   `mention_reply_latency` event before sending the next trial.
4. For warm trials, reuse the same channel session. For cold trials, issue
   `!rotate`, wait for rotation to finish, then send the fixed prompt.
5. Run at least 20 sequential trials per path. Read the last event for each path;
   its `summary` contains `windowSize` and per-stage `samples`, `p50`, `p95`, and
   `max` in milliseconds.

Keep real-provider runs scheduled or opt-in rather than merge-blocking. Provider
startup, credentials, cost, network conditions, and relay placement make those
runs useful operational evidence but unsuitable as deterministic CI.

## Privacy and current scope

Semantic latency events include IDs, path classification, durations, and sample
counts. They do not include message content, prompts, model output, credentials,
or tool arguments. Existing raw `acp_read`/`acp_write` observer events retain
their current behavior and are outside this telemetry's redaction guarantee.

The start boundary is harness receipt, not the sender's publish timestamp. The
final reply boundary is relay fanout back to the harness, not the Buzz CLI's
publish-start timestamp or a separate recipient's render timestamp. This first
instrumentation change establishes a measurable baseline; issue #2386 should
remain open until the missing outer boundaries, production baselines, explicit
budgets, and a regression job are landed.
