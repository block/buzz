# Bundled Goose macOS pilot

Internal macOS builds may enable the desktop `bundled-goose` Cargo feature.
This makes the single **Goose** (`goose`) runtime use the pinned executable
included with Buzz. Buzz Agent remains the default. Existing Goose agents use
the bundled executable on their next launch; builds without this feature keep
using the external Goose CLI. The bundled runtime supports local agents only.
Saved `goose-bundled` pilot selections load as `goose`.

## Build and package

Activate Hermit, then run `just bundled-goose`. The source revision, profile,
and feature list live in `scripts/goose-build.json`. The script fetches that
exact commit, activates its toolchain, builds with the upstream Cargo lockfile,
and stages `desktop/src-tauri/binaries/goose-acp-<target>` plus a JSON provenance
manifest. It uses an isolated `.cache/bundled-goose` checkout and target cache;
no changes to an installed Goose are needed. Supported targets are Apple Silicon
and Intel macOS. The binary is replaced atomically and checked for non-system
dynamic libraries before staging.

The lean profile includes native TLS and system-keyring so existing Goose
keychain credentials remain usable. It excludes optional bundled MCP servers,
scheduler, HTTP serving, and other extensions disabled by the upstream lean
configuration. Native developer tools and external MCP remain available.

`squareup/buzz-releases` enables `BUZZ_BUNDLE_GOOSE=1`, builds the sidecar, adds
it to its Tauri release configuration, and enables `bundled-goose`. It supplies
`BUZZ_BUILD_BUNDLED_GOOSE_PROVIDER` and `BUZZ_BUILD_BUNDLED_GOOSE_MODEL` together.
These are defaults only for the bundled runtime; structured agent/persona/global
selections, user environment values, and existing Goose file settings take
precedence. `GOOSE_PROVIDER` and `GOOSE_MODEL` environment overrides are
honored in launch, settings display, and create-agent validation. OSS builds don't enable
the feature or require the additional artifact. The internal pipeline must
select a Buzz desktop tag containing this support.

For source-tree UI development, build the sidecar and start Tauri with
`--features bundled-goose`. The development resolver finds the staged artifact;
installed apps use the executable beside Buzz, never an external PATH match.
`goose-acp` takes no `acp` subcommand. Builds without bundling continue to use `goose acp`.
Both use Goose's existing configuration and credential locations; incompatible
extensions in an existing Goose config can still cause startup errors.

## Qualification

- Run `just ci` and Tauri tests with `--features bundled-goose`.
- Test first launch with no Goose CLI installed and with an existing external
  Goose. Exactly one Goose entry with its icon must appear, and Buzz Agent must stay
  the default. Existing Goose selections must launch the bundled executable.
- Verify Databricks OAuth, model discovery, explicit provider/model/effort
  changes, and restart using the exact packaged artifact.
- Mention the agent through a real relay, perform shell/file work, and verify
  its reply lands in the right thread. Exercise cancel and a subsequent turn.
- Run the existing harness Git tests with `BUZZ_TEST_GOOSE_ACP` pointing to the
  staged executable and `BUZZ_TEST_BIN_DIR` pointing to built Buzz binaries:

  ```sh
  cargo test -p buzz-acp git_runtime_tests -- --ignored --nocapture
  ```

  With `BUZZ_TEST_GOOSE_ACP`, the Goose test uses a deterministic local provider.
  The tests verify local signed commits/tags, identity, credential scoping, and
  key cleanup. Also qualify
  authenticated relay clone/push/readback through the installed app.
- Inspect `Contents/Resources/goose-build.json` for source/build provenance. Its
  checksum is for the artifact before signing; signing changes binary bytes.
  The release pipeline checks the packaged binary before and after signing.

The pilot does not resolve upstream empty-final-response warnings after a
successful Buzz publication, shell process-tree cancellation, or unbounded
shell capture. Track these against the pinned build when collecting feedback;
bundling Goose does not switch the Buzz Agent default or establish feature parity.
