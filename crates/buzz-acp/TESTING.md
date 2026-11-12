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
