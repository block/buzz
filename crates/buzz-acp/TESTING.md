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

## Authentication failure notices

The listener recognizes Buzz Agent's `LlmAuth` ACP code (`-32001`) independently
of its message, plus the existing `Re-authenticate` and `API Error: 401` text
fallbacks for other adapters. It stops retrying the failed batch and posts a
signed kind-9 recovery notice in the original channel/thread. This uses the
existing best-effort failure-notice path and does not require Activity publishing.
The healthy listener process remains running; this is not a profile status change.

```sh
cargo test -p buzz-acp error_outcome_emission_tests -- --test-threads=1
```

These tests exercise `handle_prompt_result` and capture its signed HTTP event
with `relay_observer: false` and no observer. They cover missing Databricks
credentials, opaque structured errors, legacy text fallbacks, thread routing,
provider/CLI recovery instructions, and continued retries for non-auth failures.

For app acceptance, use a disposable agent with missing provider credentials and
Activity publishing disabled. Mention it in a thread: the first failed turn
should produce the authentication notice without retrying that batch. Sign in
through the agent's provider settings, restart the agent, and re-send the request
to verify recovery. This native workflow is separate from the local HTTP fixture.

### Integrating the shared runtime into buzz-app

`block/buzz` owns this listener. In `buzz-app`, update the `revision` in
`runtime/agent-runtime.json` to a fetchable `block/buzz` commit containing the fix,
then run `node scripts/build-agent-runtime.mjs` and rebuild the native app. The
script rebuilds all five bundled tools and regenerates their manifest/hashes;
the controller verifies the manifest against the compiled-in source revision.
Verify the staged bundle and restart existing listeners:

```sh
node scripts/verify-runtime-bundle.mjs src-tauri/resources/agent-runtime <target-triple>
```

The separate `buzz-agent` Git dependency in `src-tauri/Cargo.toml` (and its
`Cargo.lock` entry) supplies native model/auth operations. Updating only that
dependency does not update the listener. This listener-only fix needs no native
API change; aligning that dependency's revision is a separate integration choice.
Keep Activity consent unchanged and perform the native acceptance flow above.

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
