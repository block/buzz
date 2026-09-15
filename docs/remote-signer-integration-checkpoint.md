# Remote signer integration checkpoint — 2026-09-13

Work in progress, not full-app parity or a packaged release.

## State

Branch `pollen/remote-signing-integration` is based on completed local consolidation
`67d143c84`. It incorporates the complete old remote working-tree implementation
from `buzz-remote-signing` at `2a353ffe2`, preserving the new local capabilities and
compatibility corrections. The original remote tree and Fizz's preview bundle
were not modified.

Resolved areas: remote implementation of the owner capability interface; explicit
unsupported remote memory validation (no local fallback); session generation and
keyless bootstrap; bounded archive ciphertext retry; media capture; local identical
workspace reapply; committed import results and captured publication destinations;
local restart/deletion recovery behavior.

Additional work: generation-bound profile edits/deferred avatar updates with
configured-destination checks and loopback regressions; profile mutation and
observer-decrypt commands admitted by the preview dispatcher. This is not a broad
preview-gate removal. Other mutation gates remain.

## Validation before checkpoint

- Full desktop Rust workspace passed in the default/local build configuration,
  including explicitly constructed remote-session tests. Main library: 3,402 tests
  passed; existing ignored tests remain ignored. Terminal/integration/doc suites
  also passed.
- Full frontend suite: 6,511 passed; TypeScript passed before the subsequent
  Rust-only profile/gate changes.
- Rust formatting and diff checks passed.
- Initial build needed copied ignored sidecar binaries from Fizz's completed
  build. Initial compilation caught duplicate merged state fields and changed
  profile test signatures; fixed. Initial test run caught nested-runtime fixture
  setup; moved fixture creation to the blocking worker and reran full workspace.

Not yet done: full remote-build-config workspace run, clippy, affected root package
suites, final independent review, packaged integration build, live staging profile
acceptance or full-app acceptance. No claim of readiness to replace the preview.

## Resume

1. Run native workspace tests with `BUZZ_BUILD_SIGNER_MODE=remote` and
   `BUZZ_BUILD_SIGNER_API_BASE=https://test.blockstaging.build/api/goose`, then
   clippy in local and remote modes. Serialize with other Cargo activity.
2. Review generation-bound profile IPC and the new gate entries; exercise the
   registered dispatcher as well as direct command calls. The renderer wrapper
   adds its immutable realm generation via `invokeTauri`.
3. Run affected `buzz-core` and `buzz-ws-client` full suites; formatting, frontend
   checks, and differential size gates. Review the full branch delta against
   `67d143c84`, not only the conflict resolutions.
4. Independently review lifecycle/retention integration before enabling more
   workflows. Remote memory needs a backend capability; Git needs a keyless
   subprocess adapter. Do not bypass validation or remove gates wholesale.
5. Build/test a new isolated artifact only after validation. Fizz's current
   bundle remains the old remote snapshot, not this integration.

Workspace receipts: `WORK_LOGS/REMOTE_SIGNER_PR2_2026_09_13/` in `/Users/baxen/.buzz`.
Original snapshot includes full patches, tar and SHA256 manifest. No push or
installation is part of this checkpoint.

## Review boundary after separation — 2026-09-14

PR1 owns the backend-neutral captured capability lifetime, renderer authority
parsing, media read/upload scopes, profile destination capture, relay cache
identity, and common owner/query/publication admission. Local capabilities do not
expire; unsigned media recovery and best-effort local deletion remain intact.
PR2 wires Builderlab session validity and credentials into those boundaries.

Profile editing and deferred avatar updates remain enabled within the existing
remote preview allowlist. This cleanup does not remove functionality or broaden
that allowlist. Existing unsupported memory validation, Git subprocess credentials,
and workspace mutation workflows remain explicitly gated.

Archive retry stays in PR2: retaining ciphertext across service failure and fencing
its eventual commit against session replacement is remote lifecycle behavior, not
a behavior-preserving local caller refactor. Keyless initialization, enrollment,
remote transport/configuration and renderer generation propagation also stay here.

### Validation for this separation pass

Recorded working-tree runs (not a claim of full green CI):

- Frontend: 6,511 passed, zero failed.
- Remote native: 3,404 library tests passed, 22 ignored; integration suites passed.
  The command failed at doctests with missing dependency crate errors.
- Local native on PR2: 3,400 passed, four discovery/subprocess failures, 22 ignored.
- PR1 native: 3,280 passed, ten provider/discovery failures, 19 ignored.

No tests were deleted or disabled to obtain these results. Full CI, clean native
reruns and isolated app acceptance remain outstanding. This is ready for boundary
review, not a claim of merge readiness or full remote parity. No new package was
built or installed during this separation pass.
