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

`shape.thread_depth` seeds each thread as a chain: reply `k` replies to reply
`k-1`, so the thread's deepest reply sits at depth `thread_depth`. The relay
rejects depths above 100, so scenarios do too. Threads are seeded in parallel
with each other.

A scenario that cannot be scheduled is rejected when it is parsed: for
example `seed_concurrency` of 0, `human.presence_interval_secs` below 1, or a
`community_host_template` with a port while `target.connect_addr` is set (the
override maps hostnames, so HTTP would ignore it).

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

## Targeting and membership

`target.connect_addr` (`host:port`) applies only together with
`target.community_host_template`: it then replaces host resolution for the
community hosts, over both WebSocket and HTTP, while TLS names, `Host` headers
and the NIP-42 relay tag still name the community. Without a host
template, the generator connects to `ws_url` and `http_url` as written and
`connect_addr` has no effect.

Seeded identities do not join communities or channels. A relay that requires
membership rejects both `seed` and `run`.

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
An operation still in flight when the hold ends is reported as `pending`
but not judged: it has not failed yet. A relay `CLOSED` on a live
subscription counts as a `subscribe_eose` error and ends that client's
session. Each connect plus authentication must finish within
`target.connect_timeout_secs` (default 10). Quota rejections, meaning an HTTP 429 or the relay's
`rate-limited: quota exceeded` text, are reported as `throttled`, not errors.
Every other rejection, including `rate-limited: shared admission unavailable`
(the relay's admission store is down), counts as an error.
Ramp steps must increase `users`, since clients are only ever added.

When the run stops, clients get 15 seconds to settle what they have in flight
and disconnect. An operation already under way runs to its own deadline
instead of being abandoned: a connect attempt finishes within
`target.connect_timeout_secs`, and a publish waits at most 30 seconds for its
`OK`. A stalled connect therefore ends as a `connect timeout` error and a
healthy one as a success. What settles after the last step is reported in the
`shutdown` object (`ops`, `failures`, `abort_reason`) and judged by the same
limits as a step, so a connect that never completes fails the run there. A client
still busy when the grace period ends is cancelled; its in-flight operations
are recorded as `unfinished at stop` errors, and the cancellation fails the run
whatever the limits. Keep `target.connect_timeout_secs` below 15, or a stalled
connect is cancelled instead of timing out. Publishes, history fetches and
HTTP requests can outlast the grace period (30 s, 30 s and 60 s deadlines), in
which case the client is cancelled and the run fails.

The process exits non-zero when a step aborts, a client panics, a client had to
be cancelled, or `shutdown.abort_reason` is set.

`seed` exits non-zero unless the relay accepted every planned post.

`seed` provisions each community with `create_only`, so it never rotates the
owner of an existing host. A host that already exists is reused only when the
fixture owner derived from `run_id` owns it (re-running `seed` for the same
scenario); otherwise `seed` fails. Use a fresh `run_id` or a host template
with `{run}` to avoid colliding with another community.

## Safety

The generator creates real communities, channels and events. Point it only at
environments built for load tests, never at a production relay. Use a fresh
`run_id` per fixture.
