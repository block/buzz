# Relay load generator

`buzz-loadgen` (in `crates/buzz-test-client`) seeds a data shape and then
drives a population of simulated humans and agents through a ramp of user
multipliers, reporting latency percentiles, errors and rate-limit rejections
per operation for each step.

```bash
cargo run --release -p buzz-test-client --bin buzz-loadgen -- seed perf/scenarios/local-smoke.toml
cargo run --release -p buzz-test-client --bin buzz-loadgen -- run perf/scenarios/local-smoke.toml
cargo run --release -p buzz-test-client --bin buzz-loadgen -- teardown perf/scenarios/<templated>.toml
```

## Scale vectors

| Vector | Scenario knob |
|---|---|
| Communities | `shape.communities` (each provisioned through `POST /operator/communities`) |
| Channels | `shape.channels_per_community`, `population.channels_per_member` |
| History depth | `shape.history_messages_per_channel`, `human.history_limit` |
| Deep threads | `shape.thread_roots_per_channel`, `shape.thread_depth`, `human.thread_reply_fraction` |
| One hot thread | `[hot_thread]`: `seed_replies`, `authors`, `replies_per_hour`, `fetches_per_hour` |
| Humans / agents | `population.humans`, `population.agents`, multiplied by each `[[ramp]]` step's `users` |
| Throughput per client | `[human]` and `[agent]` rates |
| Reconnect churn | `human.mean_session_secs`, `population.connect_rate_per_sec` |
| Private membership | `shape.private_channels` |
| Large channels | `shape.wide_channels`, `population.wide_member_fraction` |
| Uneven membership | `population.tiers` |
| Desktop clients | `[desktop]`: `fraction`, `mode`, `windows`, `threads`, `tick_secs`, `backstop_secs`, `renewal_secs` |

`shape.thread_depth` seeds each thread as a chain: reply `k` replies to reply
`k-1`, so the thread's deepest reply sits at depth `thread_depth`. The relay
rejects depths above 100, so scenarios do too. Threads are seeded in parallel
with each other.

A scenario that cannot be scheduled is rejected when it is parsed: for
example `seed_concurrency` of 0, `human.presence_interval_secs` below 1, or a
`community_host_template` with a port while `target.connect_addr` is set (the
override maps hostnames, so HTTP would ignore it).

`[hot_thread]` models one thread that keeps growing, such as a conversation
between busy agents. `seed` posts its root in community 0, channel 0 and
then `seed_replies` replies directly under it, concurrently. During `run`,
`authors` clients (multiplied by each ramp step's `users`, like humans and
agents) reply to that root and read its newest 100 replies over HTTP. Their
replies are timed, so humans subscribed to that channel record delivery.
Leave `seed_replies` and `authors` at 0 to disable it.

Members are assigned deterministically by global index, so every shard and
`seed` agree on them without coordination:

- `shape.wide_channels` reserves the first K channels of each community as
  wide channels. Each human joins them with probability
  `population.wide_member_fraction`, on top of its own channels. Agents never
  do.
- `population.tiers` is a list of `{ fraction, channels }`. That fraction of
  humans joins `channels` channels instead of `channels_per_member`. Fractions
  sum to at most 1; the rest keep `channels_per_member`.
- A member's own channels are a run of consecutive channels after the wide
  ones, starting at its index. With no wide channels and no tiers this is the
  assignment of earlier versions.

## What each client does

- **Human**: authenticates over WebSocket and subscribes to its channels'
  messages, its mentions, typing, and ten peers' presence. On connect it
  fetches `history_limit` messages per channel. Then it sends presence
  heartbeats, typing indicators, messages (some as replies into seeded deep
  threads) and reactions on Poisson schedules.
- **Agent**: authenticates with a NIP-OA tag from its owner, polls HTTP
  `/query` (channel window plus mentions), posts messages through HTTP
  `/events`, and sends encrypted kind:24200 observer frames to its owner.
- **Desktop human**: the `desktop.fraction` of humans that model the desktop
  app instead of the basic human above. They keep the basic human's presence,
  typing, messages and reactions; the subscriptions and HTTP reads are the
  app's, described under [Desktop clients](#desktop-clients).
- **Hot-thread author**: finds the seeded hot root, then posts replies to it
  through HTTP `/events` (`hot_reply`) and reads the thread through HTTP
  `/query` (`thread_fetch`). Until the root is found every attempt counts as
  a `thread_fetch` error, so a run without the seeded thread fails.

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
`target.community_host_template`: the generator then dials the community
hosts there in plaintext, over both WebSocket and HTTP, such as a relay's
in-cluster service. Set `ws_url` and `http_url` to the community's public
scheme anyway (`wss://` and `https://` for a relay behind TLS): `Host`
headers, NIP-98 `u` tags and the NIP-42 relay tag keep that scheme, which the
relay checks against its own relay URL. Without a host template, the
generator connects to `ws_url` and `http_url` as written and `connect_addr`
has no effect.

By default channels are open and seeded identities join nothing, so a relay
that requires membership rejects both `seed` and `run`. With
`shape.private_channels = true`, `seed` creates the channels private and adds
every member: the humans and agents assigned to each channel up to the
largest ramp step, and the hot-thread root and authors in its channel. It
sends kind:9000 events of up to 100 members, `seed_concurrency` at a time,
signed by the community owner, before posting any history. A rerun adds
nobody new, and any member the relay refuses fails the seed. Seeded history
in a private channel is posted by its own members.

## Desktop clients

A desktop client models the subscriptions and reads the desktop app makes,
in two modes: `legacy`, the app before its client volume fixes, and
`current` (default), after them. Each request type is its own operation in
the report.

On a **cold start** it:

1. reads the relay's NIP-11 document (`desktop_nip11`);
2. opens its profile and membership subscriptions, and its channel
   subscriptions (`desktop_live_eose`). With up to ten channels both modes
   open one per-channel batch that replays the newest 500 events. With more,
   `legacy` opens one such batch per ten channels; `current` opens one
   live-only subscription per 2048 channels and replays the channels ten at a
   time in separate subscriptions it closes at EOSE (`desktop_replay_eose`).
   At most four are set up at once, each with a 10 s EOSE timeout;
3. reads over HTTP, at most six at a time with a 30 s timeout: its roster
   (`desktop_roster`, paged), channel names (`desktop_metadata`), sidebar
   preferences (`desktop_sidebar`), co-member profiles (`desktop_profiles`)
   and statuses (`desktop_statuses`), the custom emoji set
   (`desktop_emoji`), read state (`desktop_read_state`), unread repair in
   chunks of 128 channels (`desktop_unread`), reminders
   (`desktop_reminders`), and for each of `windows` open channels a window
   catch-up (`desktop_window`, up to five older pages while pages come back
   full) and its notices (`desktop_notices`), and for each of `threads` open
   threads a seeded root lookup and its first thread page
   (`desktop_thread_root`, `desktop_thread_window`).

Every `tick_secs` the client runs a recovery tick. In `legacy` each tick
replaces its channel subscriptions and rereads roster, read state and unread.
In `current` each of the three waits `backstop_secs` since it last ran, and
a replacement is live-only. A replacement is opened before the subscription
it replaces is closed.

Every `renewal_secs` (0 disables it; the first is jittered) the client renews
its identity and reconnects. In `legacy` that is an ordinary reconnect, which
rereads everything but channel names and sidebar preferences. In `current`
with up to ten channels the subscriptions are re-sent and nothing is read;
with more, the replays, unread repair, windows and threads run again. A
human's session end (`human.mean_session_secs`) is an ordinary reconnect in
both modes. Resumed subscriptions start 960 s before the previous session
ended, but not before 300 s before the client started.

Reads that come due while earlier ones are still running wait for them, once
each, so a slow relay does not pile reads up behind it.

## Sharding

`run --shard i/n` runs the clients whose global index `g` satisfies
`g % n == i`. Identities, community hosts and channel ids derive from
`run_id`, so shards need no coordination: run `seed` once, then start `n`
processes, such as a Kubernetes Indexed Job, with the same scenario. Each
shard reports its own clients; add counts and take the worst percentiles
across shards.

`ghcr.io/block/buzz-loadgen:main` (and `:sha-<7>`) packages the binary as the
image entrypoint, published by `.github/workflows/loadgen-image.yml` from
`Dockerfile.loadgen`. Mount the scenario and pass the subcommand as the
container arguments, for example `run /etc/loadgen/scenario.toml --shard
$(JOB_COMPLETION_INDEX)/30`. Pin `:sha-<7>` or a digest for runs you intend
to compare; `:main` moves. The container runs as UID 10001 with `/` as its
working directory and no home, so point `--report` at a writable mount.

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

## Teardown

`teardown <scenario>` removes the communities `seed` provisioned from
`target.community_host_template`, using the operator key `seed` used. For each
host it archives the community as its fixture owner, then requests owner
deletion with a request id derived from `run_id` and the host. The
relay's deletion drain does the deletion asynchronously; `teardown` reports
the requests by the stage the relay returned (`requested`), hosts this run's
fixture owner does not own (`absent`), and failures. It never touches a host
owned by anyone else, and refuses a scenario without a host template.

Re-running `teardown` replays the same request ids, so it retries failures
and reports progress without conflicting. It exits non-zero while any
community failed.

Deleted hosts stay reserved as tombstones, so a torn-down `run_id` cannot be
seeded again: `seed` fails on the first host. Give each fixture a new
`run_id`.

## Safety

The generator creates real communities, channels and events. Point it only at
environments built for load tests, never at a production relay. Use a fresh
`run_id` per fixture.
