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


## Restrictive account pilot

`BUZZ_ACP_DENY_PERMISSION_REQUESTS=true` opts into permission-request denial.
Select an explicit supported mode with `BUZZ_ACP_PERMISSION_MODE`.
The pilot uses Codex `workspace-write` or Claude `default`.
Session creation requires advertised support and a matching current value after setting the mode.
Every permission request receives a cancelled outcome; there is no automatic allow_once selection in this path.
The harness supplies no external MCP servers and suppresses generated Codex network widening.
Client terminal and filesystem operations remain unimplemented and return Method not found.
Legacy launches retain their previous behavior when the opt-in is absent.

Run the full package suite above, also clearing inherited `BUZZ_ACP_SESSION_POLICY` when its default is tested.
Regression fixtures use actual ACP stdio exchanges for grants, missing options, mode refusal and unmediated requests.
Session-creation tests inspect the actual empty MCP-server list and requested/confirmed mode.
These fixtures prove harness control flow, not vendor sandbox enforcement or authenticated account access.
The restricted pilot can prove a signed text response and shutdown; external MCP Git/Linear/Buzz CLI work is unavailable.
Claude default mode and ACP denial are not a filesystem/network sandbox.
Trusted adapter code, model-provider networking, same-UID account authority and Kubernetes administrators remain separate boundaries.
