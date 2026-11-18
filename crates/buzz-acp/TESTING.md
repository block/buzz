# Pi adapter integration

Buzz uses the [buzz-pi-acp fork](https://github.com/salman1993/buzz-pi-acp).
Install Node.js 22 or newer, configure Pi, and install the latest development
adapter from the fork's `main` branch:

```sh
npm install -g @earendil-works/pi-coding-agent
pi
npm install -g --install-links=true 'git+https://github.com/salman1993/buzz-pi-acp.git#main'
```

This test setup intentionally tracks `main`; the Desktop runtime catalog pins a
reviewed adapter revision for users.

Make sure `pi` and `buzz-pi-acp` are on PATH, then restart Buzz.

## Tests

```sh
cargo test -p buzz-acp
```

Managed agent sessions may already export harness options. Clear them when
running the package suite: three CLI parsing tests assert the unset defaults,
and inherited values would change the inputs those tests exercise. Running the
package serially also avoids scheduling flakes in existing short-deadline tests.

```sh
env -u BUZZ_ACP_ALLOWED_RESPOND_TO \
  -u BUZZ_ACP_LAZY_POOL \
  -u BUZZ_ACP_IDLE_POOL_SLEEP \
  cargo test -p buzz-acp -- --test-threads=1
```

Run the ignored real-adapter test with a built fork checkout:

```sh
BUZZ_TEST_PI_ACP=/absolute/buzz-pi-acp/dist/index.js \
  cargo test -p buzz-acp real_pi_preserves -- --ignored
```

## Observer retention measurement

The observer has deterministic regression tests for replay byte/count limits,
oversized payloads and envelopes, and a stalled live subscriber. Run them with:

```sh
cargo test -p buzz-acp --lib observer::tests -- --test-threads=1
```

An ignored local probe measures the actual `ObserverHandle::emit`, `snapshot`
and broadcast paths without a relay or agent. It defaults to the #7987 workload:
1,000 events with a 1 MiB ASCII string each and a subscriber that does not drain
until after emission. It prints JSON with `emitMs`, replay/live serialized byte
totals and event counts, and the subscriber's lag count.

```sh
cargo test --release -p buzz-acp --lib observer::tests::retention_probe -- \
  --exact --ignored --nocapture

# Small-event throughput control, including ordinary count-based eviction:
BUZZ_OBSERVER_PROBE_EVENTS=10000 BUZZ_OBSERVER_PROBE_BYTES=1024 \
  cargo test --release -p buzz-acp --lib observer::tests::retention_probe -- \
  --exact --ignored --nocapture
```

Use fresh processes, the same profile and workload, an idle machine, and repeated
alternating baseline/candidate runs for timing comparisons. There is no timing
threshold. `emitMs` includes payload construction and excludes the later snapshot
and receive/serialization measurement. Serialized byte totals are not heap/RSS
measurements. On macOS, `/usr/bin/time -l` around the test executable printed by
`cargo test --release -p buzz-acp --lib --no-run` can additionally measure peak
RSS; do not time Cargo compilation as part of the memory workload. The probe's
snapshot and receive measurements allocate their own copies, so its peak RSS
includes measurement overhead.

Replay keeps the newest suffix within both 1,000 events and 4 MiB of serialized
JSON. Events are fitted with the existing relay payload elision before either
store retains them. Large-payload preparation and snapshot copies stay outside
the replay lock. Publication finalizes the sequence and timestamp together, and
byte accounting uses that finalized envelope. The live ring preserves its requested 1,000-event capacity
(Tokio rounds it to 1,024 slots); its per-event relay-size cap bounds retained
serialized JSON to less than 64 MiB. These are JSON budgets, not exact allocator
limits. Oversized telemetry carries the existing elision markers; an envelope
that cannot fit even after payload elision is dropped with a warning and sequence
gap. A lagging receiver continues to report its dropped-event count.

## Git bootstrap

`cargo test -p buzz-acp --test git_bootstrap` starts the actual harness with a
probe adapter, runs real signed commits/tags and scoped credential resolution,
and verifies key cleanup on startup failure and SIGTERM. No relay is contacted.

To exercise the real runtime boundaries on Unix:

```sh
cargo build -p buzz-acp -p buzz-agent -p buzz-dev-mcp
BUZZ_TEST_BIN_DIR="$PWD/target/debug" cargo test -p buzz-acp git_runtime_tests -- --ignored --nocapture
```

The Buzz Agent test uses a deterministic local OpenAI-compatible response to
invoke the actual MCP shell. The Goose test requires an installed, configured
Goose and uses its provider to invoke the native developer shell. Both operate
only on temporary local repositories, verify commit/tag signatures and identity,
check unrelated-remote credential scoping, and assert keyfile removal. They do
not replace authenticated relay clone/push/readback testing.

## Observer performance measurement

An ignored local probe exercises the production publish queue and its JSON
fitter. The default workload is 200 events, each with 1 MiB across 64 string
leaves. It prints elapsed milliseconds and retained queue bytes/events; there
is no timing threshold or additional CI job.

```sh
cargo test --release -p buzz-acp --lib observer::perf_tests::publish_queue_probe -- \
  --exact --ignored --nocapture

# Under-budget control:
BUZZ_OBSERVER_PROBE_EVENTS=10000 BUZZ_OBSERVER_PROBE_BYTES=1024 \
  BUZZ_OBSERVER_PROBE_FIELDS=1 \
  cargo test --release -p buzz-acp --lib observer::perf_tests::publish_queue_probe -- \
  --exact --ignored --nocapture
```

Elapsed time includes cloning the template payload and queue ingestion; it
excludes template construction and later publication/encryption. Use release
builds, fresh processes, identical workloads, and repeated alternating runs
when comparing revisions. Queue bytes are serialized JSON, not heap or RSS.

## Observer frame packing measurement

To isolate frame packing, the drain probe queues 10,000 small events before
starting its clock, then packs every event into frames. It checks complete FIFO
delivery, frame byte limits and empty byte accounting, and reports elapsed
milliseconds and frame count. `BUZZ_OBSERVER_PROBE_EVENTS` changes the count;
the workload must fit the existing queue budget.

```sh
cargo test --release -p buzz-acp --lib observer::packing_tests::publish_queue_drain_probe -- \
  --exact --ignored --nocapture
```

This measures packing only, excluding ingestion, publish pacing, encryption,
relay delivery and rendering. It does not estimate overall app latency.
