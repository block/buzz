# Command Console Phase 1 foundation

Phase 1 adds a discoverable, read-only Command Console to the Buzz desktop app.
It establishes classification contracts, truthful local status, deterministic
development services, and recoverable local workspace data. It does not run
advisers or connect any later-phase data source.

Continue with the [Phase 2 local agent runtime](phase-2-local-agent-runtime.md)
for the separately packaged LM Studio-native adviser transport, its security
boundary, and verified local-runtime evidence.

Open **Command Console** from the pinned desktop sidebar. The screen always
shows the current boundary:

- new command artefacts default to `OFFICIAL`; `PUBLIC` must be selected
  explicitly;
- Chief of Staff, Operations, Navigation, Daily Routine, Reporting, and Plans
  are placeholders marked **Not yet operational**;
- relay and local-compute cards report only observed state; unknown or
  incomplete probes are never labelled healthy;
- LM Studio, Memory, RAG, and Apple inputs remain **Not configured**.

`OFFICIAL` is an information-handling default, not encryption, access control,
or permission to disclose material. Existing Buzz identity, relay membership,
event signing, and host-derived community boundaries remain authoritative.

## Prerequisites and local launch

Activate the repository's pinned Hermit environment before running commands:

```bash
. ./bin/activate-hermit
```

For a normal development stack, install and start Docker Desktop, then use:

```bash
just setup
just dev
```

`just setup` starts PostgreSQL, Redis, MinIO, and optional development services.
The Keycloak 26 readiness check uses its enabled management health endpoint on
port 9000. Docker Desktop is required for this service-backed path, but not for
the standalone desktop path.

To exercise only the desktop application, with no relay, database, migrations,
or Docker startup:

```bash
just fresh=1 desktop-standalone
```

`fresh=1` removes only the current development instance's webview state and
development keyring entries. It refuses production bundle identifiers and does
not touch relay or database data. With no saved community, the supported UI is
community selection; add a community later when a relay is available. A first
source launch can still download or compile missing Hermit, Rust, pnpm, and
Tauri dependencies. Buzz also attempts to populate its speech-to-text and
text-to-speech model caches in the background on first launch. Those downloads
are not required for frontend startup: the frontend still loads when they fail,
but speech features remain unavailable. Cache the toolchain and speech models
first if the Mac must be fully air-gapped.

## macOS build and packaging status

Phase 1 is a source and development foundation inside the existing Tauri 2
desktop app. It does not add or require a separate Xcode project. Local
development uses `just desktop-standalone`. The repository's existing unsigned
bundle recipe is `just desktop-release-build`, but Task 6 did not exercise or
verify that packaging path.

Packaging remains prerequisite and deferred evidence on the Task 6 host. Only
Command Line Tools were selected, and `xcodebuild -version` failed:

```text
xcode-select: error: tool 'xcodebuild' requires Xcode, but active developer directory '/Library/Developer/CommandLineTools' is a command line tools instance
```

A Mac producing a bundle must have the full Xcode application installed,
accepted, and selected so `xcodebuild -version` succeeds. Signed and notarized
release DMGs continue to use the repository release workflows and credentials
documented in [RELEASING.md](../../RELEASING.md); Phase 1 does not change that
process. These are repository capabilities and prerequisites, not Task 6
packaging verification.

## Security boundary

Phase 1 deliberately has no adviser execution, model invocation, retrieval,
Memory MCP replication, Apple Calendar/Reminders/Notes access, or proposed
workspace mutation. It has no integration with ship control, navigation
control, communications, combat, logistics, or personnel systems.

Command-domain parsers accept only versioned exact shapes, preserve the
classification ceiling of nested artefacts, and bound untrusted JSON
validation. UI status is read-only. A service is shown as connected only after
a successful current probe; stale, failed, client-only, or unverified local
compute is unavailable rather than healthy.

Keep secrets out of command artefact content and source references. Use the
repository's normal secret stores and follow [SECURITY.md](../../SECURITY.md)
for vulnerability reporting.

## Backup and restore

Back up local PostgreSQL and MinIO data to an existing absolute directory
outside every checkout and linked worktree:

```bash
just backup-local-workspace /Users/me/BuzzBackups
```

The timestamped backup is permission-restricted, carries a versioned manifest
and SHA-256 inventory, and excludes `.env`, signing keys, desktop keychains,
Redis, and other secret or ephemeral state. A failed backup has no completed
manifest and is not restorable.

Restore only while Buzz writers are stopped:

```bash
just restore-local-workspace \
  /Users/me/BuzzBackups/buzz-local-workspace-20260724T010203Z
```

Restore refuses repository-contained paths and symbolic links, validates the
source, copies it into a private staging directory, validates the staged copy,
and only then asks for the exact confirmation `RESTORE`. It replaces database
objects and removes MinIO objects absent from the snapshot. Read the complete
[local workspace backup and restore runbook](../development/local-workspace-backup.md)
before using it.

## Acceptance evidence

Aggregate `just ci` passes on the verification host. The first attempt appeared
to stall at `mobile-check`, but diagnosis showed that Hermit was silently
downloading the pinned Flutter 3.41.7 SDK (a roughly 1.6 GB first-run
prerequisite), rather than deadlocking. After that verified toolchain download,
mobile formatting and analysis passed, all 541 Flutter tests passed, and the
complete aggregate gate was rerun successfully.

The following focused and integration gates also passed:

- root unit tests;
- desktop type checking, repository checks, Node tests, and production build;
- desktop Tauri compile checks and unit tests;
- web production build;
- mobile formatting, analysis, and all 541 Flutter tests;
- Compose configuration validation and Keycloak health;
- the mocked local-workspace backup and restore regression suite; and
- the focused Command Console E2E described below.

The mocked Command Console browser regression opens the route through the same
pinned sidebar control used by a person. It verifies `OFFICIAL`, all six adviser
placeholders, relay `Unavailable`, local compute `Offline`, four
`Not configured` capabilities, and the absence of a `Connected` claim:

```bash
(cd desktop && pnpm build:e2e && \
  pnpm exec playwright test --project=smoke command-console.spec.ts)
```

The spec waits for animations and writes
`desktop/test-results/command-console/phase-1-foundation.png`. When attaching
that dedicated directory to a pull request, run the repository-hosted workflow
from the repository root:

```bash
./scripts/post-screenshots.sh <PR-number> \
  desktop/test-results/command-console
```

Do not use the relay media endpoint or a third-party image host for PR
screenshots.

On 24 July 2026 the standalone recipe was also exercised with relay environment
variables unset, outbound HTTP(S) and `ALL_PROXY` directed to a closed local
port, and `docker` replaced by a fail-closed shell shim. The Tauri runtime and
Vite client loaded after both speech-model requests failed, and the Docker shim
was never invoked. This verifies that unavailable network services do not gate
the standalone frontend and that the recipe does not start Docker. The Mac was
locked during this run, so the native community-selection screen could not be
visually inspected; that final native-state observation remains incomplete
rather than being inferred from frontend startup.

## Deferred phases

Later phases may add adviser orchestration, local model routing through LM
Studio, RAG snapshots and replication, Memory MCP integration, Apple input
adapters, scheduled briefs, and approval-gated workspace actions. None of those
capabilities is simulated or partially enabled by Phase 1. Each needs its own
security review, tests, operational failure states, and explicit approval
boundary before being presented as available.
