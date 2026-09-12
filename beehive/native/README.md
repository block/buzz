# Databricks native helper (macOS arm64 prototype)

This is a callable **Rust binary**, `beehive-databricks`, linked against this
checkout's `buzz_agent` library. No OAuth implementation lives in JS. The helper
uses `databricks_oauth_config`, `PkceOAuthTokenSource::new_with_custody` and
`discover_databricks_models_with_token_source`. Node sends one bounded JSON
request over owned pipes; secrets never enter argv or stderr. Do not invoke the
helper to inspect credentials. It refuses terminal stdin/stdout.

## Reproducible package step

From the exact committed checkout on macOS arm64 (no install or global changes):

```sh
ARTIFACT=/absolute/fresh/artifact-directory
RUST="$HOME/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin"
mkdir -p "$ARTIFACT/helper-package/bin"
PATH="$RUST:/usr/bin:/bin" CARGO_TARGET_DIR="$ARTIFACT/target" \
  "$RUST/cargo" build --locked --release -p buzz-agent --bin beehive-databricks
install -m 700 "$ARTIFACT/target/release/beehive-databricks" \
  "$ARTIFACT/helper-package/bin/beehive-databricks"
COPYFILE_DISABLE=1 tar -C "$ARTIFACT/helper-package" -czf \
  "$ARTIFACT/beehive-databricks-macos-arm64.tar.gz" bin/beehive-databricks
shasum -a 256 "$ARTIFACT/helper-package/bin/beehive-databricks" \
  "$ARTIFACT/beehive-databricks-macos-arm64.tar.gz"
```

Merge `helper-package/bin/beehive-databricks` into the **Beehive package's** `bin/`
when preparing each machine's next export. Do not overwrite Buzz Desktop,
buzz-agent, or any other installed product. Existing Node 24.15.0 / Bun 1.4.2
packaging stays unchanged. No source-time or runtime download/build fallback.
The binary is built for arm64; other platform custody/export is not implemented.

## Custody and lifecycle

* Exact `beehive` OS generic-password entry, account `provider:<uuid>`, containing
  the workspace-bound access/refresh bundle. Native Security Framework writes
  and exact read-back precede public provider CAS. No Desktop services or token
  files. The existing public recovery-reference log precedes attempted login.
* Browser sign-in is explicit. The shared OAuth owner supplies a random CSRF
  state nonce, PKCE S256 verifier, loopback ephemeral callback, state rejection,
  grant exchange, expiry and refresh. This is OAuth authorization-code, not an
  OIDC identity-token login; no unvalidated ID-token identity claim is consumed.
  Discovery and grant endpoints must share the validated workspace origin;
  external-custody OAuth HTTP redirects are disabled.
* Same-account helper processes hold an OS-released advisory lock across load,
  grant rotation and verified storage. Only lock/attempt metadata lives in the
  owner-only temporary coordination directory. Per-invocation files are removed
  after awaited close. Node deadlines: login 180s, models/token 20s; cancellation
  sends TERM, then KILL after 250ms, and awaits close before settling.
* Model listing and Start preflight are headless and can refresh, never opening
  the browser. For catalog OS-backed Databricks runtimes, `spawnAgent` now starts
  the run-owned Node `databricks-runtime-child.ts` wrapper. The stock harness
  retains `databricks_v2` and its native wire routing, but receives an ephemeral
  loopback endpoint and random per-run bearer capability, not the provider token.
  Each model/catalog request calls this unchanged native helper's `token` action
  against the immutable real workspace/reference. Native expiry/refresh/verified
  rotation therefore serves an ongoing run without restarting the agent.
* The proxy admits only canonical v2 catalog/model paths and the selected model;
  no redirects or arbitrary upstream hosts. OAuth/network failures fail closed;
  upstream 401/403 latches rejection for that run (explicit sign-in/new run may be
  needed). It does not implement a parallel forced-refresh OAuth engine. Native
  refresh is expiry-based; a server-side early rejection is not silently retried
  with another provider/workspace. No streaming extension is claimed: current
  canonical Buzz Agent requests/responses are non-streaming.
* Wrapper, harness and native helpers belong to the existing supervised process
  group. Stop fences responses, aborts fetch/helpers, awaits close, and retains
  the supervisor's existing TERM/KILL/absence verification. Legacy external-OAuth
  and explicit token-file bindings are unchanged; only OS-backed catalog bindings
  use the adapter. No helper binary change/rebuild is needed for this integration.
* Cancellation may follow an OS write. It cannot undo grant rotation or pretend
  rollback; no cancelled operation commits a new public provider. Recovery refs
  remain. Browser windows owned by the OS may remain open; helper/listener do not.

Tests explicitly substitute native/browser/OS custody with synthetic seams.
`manager-installed-loader.ts` blocks this new native path in future install
smokes. Fresh HOME alone is not credential or browser isolation.

## Export additions for runtime integration

Include every `beehive/src/*.ts` **and `beehive/src/*.json`** in the Node/Bun
package, especially both runtime adapter modules, `harness-contract.json` and
`model-capabilities.json`. The latter is a byte-identical packaged copy of
`scripts/model-capabilities.json`, checked by tests. Regenerate the harness
contract explicitly with `python3 beehive/tools/export-harnesses.py <reviewed-SHA>`
from the repository root; its current source is Desktop main
`78618804ec86a014524ad7d1fb55928e8f5c3edf`. Keep the existing helper binary in
`bin/beehive-databricks`; do not build/install anything at runtime.
