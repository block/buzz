# Buzz relay bus scaling harness

This harness gives reproducible evidence for the rewrite's Redis fan-out scaling claim:

- **old/global bus:** every relay pod receives every community's event;
- **new/community-scoped bus:** each pod retains only the server-resolved community topics for which it has local subscribers (`buzz:{community_id}:global` or `buzz:{community_id}:channel:{channel_id}`).

Run:

```bash
./perf/relay_bus_scaling.py
```

Baseline scenario used for the PR summary:

```text
64 communities × 100 events/s, one subscribed community, all pods interested in that community
```

Current output:

| pods | old global cluster ingress/s | old avg pod ingress/s | new scoped cluster ingress/s | new avg pod ingress/s | reduction | old irrelevant/pod | new irrelevant/pod |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 6,400 | 6,400 | 100 | 100 | 64.0× | 98.44% | 0.00% |
| 2 | 12,800 | 6,400 | 200 | 100 | 64.0× | 98.44% | 0.00% |
| 4 | 25,600 | 6,400 | 400 | 100 | 64.0× | 98.44% | 0.00% |

The code path this corresponds to is `buzz_pubsub::EventTopicKey::redis_channel()` (`crates/buzz-pubsub/src/topic.rs`), with `retain_topic` / `release_topic` driving dynamic local Redis `SUBSCRIBE` interest.

This isolates the bus-bound scaling property. Live relay latency, DB capacity, and client rendering should be measured separately with a full stack because they include unrelated bottlenecks.
