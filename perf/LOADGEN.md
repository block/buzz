# Relay load generator

`buzz-loadgen` (in `crates/buzz-test-client`) seeds a data shape and then
drives a population of simulated humans and agents through a ramp of user
multipliers, reporting latency percentiles, errors and rate-limit rejections
per operation for each step.

```bash
cargo run --release -p buzz-test-client --bin buzz-loadgen -- seed perf/scenarios/local-smoke.toml
cargo run --release -p buzz-test-client --bin buzz-loadgen -- run perf/scenarios/local-smoke.toml
```

## Scale vectors

| Vector | Scenario knob |
|---|---|
| Communities | `shape.communities` (each provisioned through `POST /operator/communities`) |
| Channels | `shape.channels_per_community`, `population.channels_per_member` |
| History depth | `shape.history_messages_per_channel`, `human.history_limit` |
| Deep threads | `shape.thread_roots_per_channel`, `shape.thread_depth`, `human.thread_reply_fraction` |
| Humans / agents | `population.humans`, `population.agents`, multiplied by each `[[ramp]]` step's `users` |
| Throughput per client | `[human]` and `[agent]` rates |
| Reconnect churn | `human.mean_session_secs`, `population.connect_rate_per_sec` |

## What each client does

- **Human**: authenticates over WebSocket and subscribes to its channels'
  messages, its mentions, typing, and ten peers' presence. On connect it
  fetches `history_limit` messages per channel. Then it sends presence
  heartbeats, typing indicators, messages (some as replies into seeded deep
  threads) and reactions on Poisson schedules.
- **Agent**: authenticates with a NIP-OA tag from its owner, polls HTTP
  `/query` (channel window plus mentions), posts messages through HTTP
  `/events`, and sends encrypted kind:24200 observer frames to its owner.

Messages carry their send time, so every subscriber records publish→delivery
latency (`deliver`). Across machines this depends on clock sync.

## Modeling a deployment

The `[human]` and `[agent]` defaults are round placeholders. To model a known
deployment, divide its hourly event counts by its client counts and set the
results as the scenario's rates. Then a ramp step's `users` reads as a
multiple of that deployment's population. Keep such scenarios with that
deployment's own configuration.

## Sharding

`run --shard i/n` runs the clients whose global index `g` satisfies
`g % n == i`. Identities, community hosts and channel ids derive from
`run_id`, so shards need no coordination: run `seed` once, then start `n`
processes, such as a Kubernetes Indexed Job, with the same scenario. Each
shard reports its own clients; add counts and take the worst percentiles
across shards.

## Abort

Each step reports every operation from the start of its ramp to the end of
its hold, so connection failures during the ramp count. At the end of the
hold, the run stops if any operation's p99 (publish→delivery included)
exceeds `abort.max_p99_ms`, if any operation other than delivery has an
error rate above `abort.max_error_rate`, or if a client exited unexpectedly.
A relay `CLOSED` on a live subscription counts as a `subscribe_eose` error
and ends that client's session. Each connect plus authentication must finish
within `target.connect_timeout_secs` (default 10). Rate-limit rejections are
reported as `throttled`, not errors. Ramp steps must increase `users`, since
clients are only ever added. The process exits non-zero when a step aborts
or a client panics.

`seed` exits non-zero unless the relay accepted every planned post.

## Safety

The generator creates real communities, channels and events. Point it only at
environments built for load tests, never at a production relay. Use a fresh
`run_id` per fixture.
