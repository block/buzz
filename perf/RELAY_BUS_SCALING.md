# Buzz relay bus scaling harness

This harness gives reproducible evidence for the rewrite's Redis fan-out scaling claim:

- **old/global bus:** every relay pod receives every community's event;
- **new/community-scoped bus:** each pod retains only the server-resolved community topics for which it has local subscribers (`buzz:{community_id}:global` or `buzz:{community_id}:channel:{channel_id}`).

The measured default uses Redis PUB/SUB directly with simulated relay pod subscribers. It intentionally isolates the bus boundary: no DB ingest, websocket framing, client rendering, or relay business logic is included.

## Run the measured Redis harness

Start Redis locally, or point at an existing instance with `REDIS_URL`:

```bash
REDIS_URL=redis://127.0.0.1:6379/0 ./perf/relay_bus_scaling.py --mode redis
```

`--mode redis` is the default. The script uses only Python stdlib and speaks RESP directly; no Python Redis dependency is required.

Baseline scenario used for the PR summary:

```text
64 communities × 100 events/s, one subscribed community, all pods interested in that community, pods = 1,2,4
```

Measured output shape:

| pods | old global cluster ingress/s | old avg pod ingress/s | new scoped cluster ingress/s | new avg pod ingress/s | reduction | old irrelevant/pod | new irrelevant/pod |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 6,400 | 6,400 | 100 | 100 | 64.0× | 98.44% | 0.00% |
| 2 | 12,800 | 6,400 | 200 | 100 | 64.0× | 98.44% | 0.00% |
| 4 | 25,600 | 6,400 | 400 | 100 | 64.0× | 98.44% | 0.00% |

The harness fails non-zero by default unless:

- observed reduction is at least 95% of the ideal `communities / subscribed_communities`; and
- scoped-mode irrelevant delivery is at most `--max-scoped-irrelevant-pct` (default `0.0`).

That makes the scaling claim load-bearing: if scoped subscribers are accidentally changed to receive the old global firehose, the assertion goes red.

## No-service model mode

For quick review without Redis:

```bash
./perf/relay_bus_scaling.py --mode model
```

Model mode prints the same contract using deterministic arithmetic. It is useful for docs and unit tests, but the PR evidence should cite `--mode redis` because that path measures actual Redis PUB/SUB delivery.

## Unit tests

```bash
python3 -m unittest discover -s perf -p 'test_*.py'
```

The unit tests pin the default 1/2/4-pod 64× contract and include a mutant row that represents scoped mode receiving irrelevant global-firehose traffic; that row must fail the assertion.

## Code provenance

The scoped Redis channel format corresponds to `buzz_pubsub::EventTopicKey::redis_channel()` in `crates/buzz-pubsub/src/topic.rs`:

- global: `buzz:{community_id}:global`
- channel: `buzz:{community_id}:channel:{channel_id}`

`retain_topic` / `release_topic` drive dynamic local Redis `SUBSCRIBE` interest. This harness measures that bus-bound property only. Live relay latency, DB capacity, and client rendering should be measured separately with a full stack because they include unrelated bottlenecks.
