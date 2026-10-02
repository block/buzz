# Mobile performance diagnostics

With the repository's Hermit environment active, run from `mobile/`:

```sh
flutter test --dart-define=BUZZ_RUN_BENCHMARKS=true test/benchmark/channel_live_burst_test.dart
```

The diagnostic is skipped by default. It does not use a relay, credentials, or
community content.

The buffered-channel diagnostic sends synthetic events through the real
`RelaySessionNotifier` event buffer and `ChannelMessagesNotifier` live callback.
Only connection state and history responses are replaced. It measures one
synchronous flush and a zero-delay timer queued just before that flush. The timer
shows how long other event-loop work waits behind the consumer callbacks.

It varies retained history (50, 500, 2000 events) and live burst size (50, 200,
1000 events). Each case warms up twice before emitting four JSON measurement
records in microseconds. Compare medians on the same host under similar load.
No wall-clock threshold is asserted because debug/JIT compilation and host load
make it unsuitable for a stable CI performance gate. Final event counts are
asserted to ensure the measured work actually completes.

This is a bounded stress diagnostic, not a representative release benchmark.
Retained history is modeled as one synthetic page instead of real pagination.
It excludes widgets, media, network traffic, signature verification, and other
subscriptions. It can identify synchronous consumer costs, but cannot establish
that a measured burst occurred in a real community or predict release frame
latency.
