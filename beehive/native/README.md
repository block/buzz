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
  the browser. Launch passes the refreshed access token through the existing
  private `DATABRICKS_TOKEN` harness environment and the exact workspace/model.
  **A running stock buzz-agent receives a static access token**: long-lived
  in-process refresh after that token expires is not provided by this bridge.
  An explicit new Start/Restart obtains a refreshed token. This is a known
  remaining limitation, not continuous token-source integration in the harness.
* Cancellation may follow an OS write. It cannot undo grant rotation or pretend
  rollback; no cancelled operation commits a new public provider. Recovery refs
  remain. Browser windows owned by the OS may remain open; helper/listener do not.

Tests explicitly substitute native/browser/OS custody with synthetic seams.
`manager-installed-loader.ts` blocks this new native path in future install
smokes. Fresh HOME alone is not credential or browser isolation.
