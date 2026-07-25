# Command Console Phase 4 Daily Command Brief

Phase 4 turns the local macOS Command Console into a scheduled, evidence-cited
Daily Command Brief. Five specialist advisers run against one frozen OFFICIAL
source snapshot. A tool-free Chief of Staff then consolidates their validated
contributions without removing dissent or adding unsupported claims.

The application remains advisory and non-accredited. It does not make
navigational decisions, create executable navigation orders, control ship
systems, or authorise workspace actions. Proposed actions remain visibly
`pending`.

## Local-only execution boundary

Every Phase 4 brief is classified `OFFICIAL`. Adviser generation uses the
MacBook LM Studio native API with only catalogue-admitted literal-loopback
Memory and RAG MCP services. There is no cloud fallback. The egress and source
policies reject non-loopback service endpoints, pseudo-tool calls written as
reasoning text, retrieved instructions, mixed snapshots, unsupported Chief of
Staff claims, and non-pending actions.

The default specialist concurrency is one and the only permitted alternative
is two. The Chief of Staff runs after all five specialists and receives no
tools. A source failure degrades the affected section; it never grants
permission to fabricate evidence or silently substitute a cloud provider.

The complete brief contains these nine sections:

1. Today at a glance
2. Operational priorities and risks
3. Navigation considerations
4. Daily routine and calendar
5. Reports and returns due
6. 30, 60 and 90 day planning horizon
7. Decisions required
8. Conflicts and gaps
9. Sources

Every displayed factual finding cites an entry in the bounded source ledger.
The ledger exposes provenance metadata and freshness, not the retrieved passage
or hidden model context. One brief is bound to exactly one admitted snapshot
ID. Stale sources, permission denial, missing information, Memory conflicts,
degraded sections, limitations, and dissent remain visible.

## Schedule and recovery semantics

The native scheduler owns the fixed schedule identity
`daily-command-brief`. Its default is enabled at `06:00` in the current
macOS IANA timezone, with same-day catch-up enabled and concurrency one.
Renderer input may change only enabled state, local time, and concurrency; it
cannot replace the trusted schedule identity or timezone.

A scheduled run acquires a durable unique claim for the schedule and local
date before generation. Its run identity is deterministic. Concurrent timers,
restart, and verified macOS wake reconciliation therefore cannot start a
second run for the same date. Startup or wake may perform at most one same-day
catch-up when enabled. A missed prior day is not replayed.

If identity, model, admission capacity, or required local state is unavailable,
the claim is retained in a visible deferred state. Only a bounded, trusted
readiness transition can retry it. Recovery first reconciles the exact run ID
against the in-process registry and encrypted terminal spool; it does not
invent a replacement run. Manual generation uses a separate explicit run ID
and does not acquire or modify the daily claim.

## Source and readiness diagnostics

The Command Console reports the active Buzz relay, local compute, LM Studio,
Memory, RAG, and Apple-input status. A service is admitted only after its
native validator succeeds:

- LM Studio must be a literal-loopback native API with the selected model and
  structured-output behavior.
- Memory must have protected credentials, expected node identity, an allowed
  read-only adviser tool, valid immutable revision evidence, and no unresolved
  conflicted field used by the brief.
- RAG must have the expected signed active snapshot, service/model revisions,
  catalogue, point counts, freshness, and authenticated read-only tools.
- Apple inputs remain read-only and allowlisted. Permission denial or stale
  data is reported per section and fails soft.

Readiness means that the named service answered its probe. Admission is the
stronger result required before its evidence or tools enter a brief.

## Signed local history

Every terminal lifecycle result is converted to a bounded kind `44210`
NIP-CB event. The owner signs it and NIP-44-v2 encrypts it to the same owner
key. The envelope has exactly one owner `p` tag and no community tag.

The encrypted event is committed to the local SQLite spool before relay
publication. Relay failure leaves the exact signed event queued; reconnect
retries that same event ID rather than re-signing. Local restart reloads the
latest valid owner history from the spool.

Relay policy permits only a valid owner-authored envelope. REQ and COUNT are
owner-only, ID lookup still applies the owner gate, and kind `44210` is omitted
from full-text indexing and search results. Ciphertext is not a substitute for
these authorization checks.

## Hermetic acceptance

Activate the checked-in toolchain and run:

```bash
. ./bin/activate-hermit
just check-daily-command-brief
```

This default gate is hermetic. It runs the LM Studio structured-tool and
pseudo-tool fixtures; adviser, source, orchestration, schedule, spool, audit,
Tauri command, core wire, relay authorization, backup/restore, React, and
Swift helper tests. It prints explicitly that live probes were skipped.

The orchestration contract itself is checked with:

```bash
bash scripts/tests/check-daily-command-brief-test.sh
```

That test proves the Just entrypoint, exact authoritative child suites,
non-zero Rust test selection, failed-child propagation, success-claim
suppression, and fail-closed live configuration. The child binaries are mocked
there; the default gate above runs their real production fixture suites.

The wider regression gates are:

```bash
just check-command-knowledge
just ci
pnpm -C desktop run build:e2e
cd desktop
pnpm exec playwright test --project=smoke
```

Run the Swift helper independently when investigating Xcode failures:

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  xcodebuild test \
    -project desktop/apple-inputs/BuzzAppleInputs.xcodeproj \
    -scheme BuzzAppleInputs \
    -destination 'platform=macOS' \
    CODE_SIGNING_ALLOWED=NO
```

### Task 9 verification record

On 25 July 2026, the corrected Task 9 hermetic runner and aggregate `just ci`
passed. The runner's authoritative Rust filters all selected non-zero tests,
including the provenance, strict brief-wire, and agent evidence-admission
suites omitted by the original gate. It also passed 3544 desktop tests and the
18-test Apple-input suite. The two Daily Command Brief E2E scenarios passed
with assertions for the visible advisory limitation, pending proposal, and
citation-to-ledger focus, retrieval timestamp, and page location.

The first corrected full desktop smoke run remained non-green: 697 passed, 1
skipped, and 2 failed in 17.5 minutes. The supported-link-preview failure
measured a detached optimistic card once. A deterministic replacement-during-
measurement regression then reproduced the exact 16px-versus-0px failure. The
shared radius helper now polls the current locator without weakening its 16px
or smooth-corner assertions. The real scenario plus regression passed 40/40,
and all 57 helper consumers passed. The failed-initial-relay-dial scenario
failed 5/10 because its state poll ran before the E2E seam was installed; a
narrow wait for that seam passed 20/20.

The next aggregate passed those corrected cases but exposed two separate
readiness races. The channel browser Enter scenario now waits until the visible
create row reflects the live query, and the create-dialog helper requires a
prior Radix exit-animation node to detach before dispatching a new open
shortcut. Those two scenarios passed 100/100 under parallel repetition. The
final full desktop smoke run passed with 700 passed, 1 skipped, and 0 failed in
17.9 minutes.

The inherited video-review test now asserts its durable `1 new message`
affordance when the virtualized row is intentionally offscreen. Playback-rate
and thread-comment responsibilities were retained in focused scenarios instead
of being hidden inside the 543-line review path. The original and two extracted
scenarios passed 15/15 across five repetitions and passed in the aggregate run.

`PHASE4-SMOKE-001` is closed on draft PR #4 by deterministic RED/GREEN evidence,
focused repetition, and the green 700-pass aggregate. No acceptance waiver was
used. Independent review then approved the exact post-review delta at
`d50cc1a6` with no findings after fresh focused verification. Phase 4 code and
fixture acceptance is complete.

## Controlled live offline exercise

Live testing is intentionally opt-in. It requires reviewed services already
bound to literal IPv4 loopback and an operator-owned executable driver:

```bash
BUZZ_DAILY_BRIEF_LM_STUDIO_URL=http://127.0.0.1:1234 \
BUZZ_DAILY_BRIEF_LM_STUDIO_MODEL='<reviewed-model-id>' \
BUZZ_DAILY_BRIEF_MEMORY_URL=http://127.0.0.1:18006/mcp/ \
BUZZ_DAILY_BRIEF_RAG_URL=http://127.0.0.1:8005/mcp/ \
BUZZ_DAILY_BRIEF_LIVE_DRIVER=/absolute/path/to/reviewed-live-driver \
  ./scripts/check-daily-command-brief.sh --live
```

The runner rejects missing values, hostnames, LAN addresses, invalid ports,
wrong MCP paths, symlinks, and non-executable or relative drivers. It first
runs a tool-free LM Studio native API health smoke, then invokes the reviewed
driver with the three validated loopback URLs and the validated model ID.

The runner does not itself prove structured MCP calls, five specialist
executions, signed-application provenance, offline egress isolation, resource
limits, or signed-history reload. Those claims belong to the reviewed driver
and its retained evidence. A zero driver exit means only that the reviewed
driver completed; the operator must still inspect its evidence before recording
the controlled live exercise as passed.

The exercise is tracked as `PHASE4-COMMISSION-001` in this runbook, the Task 9
report, and draft PR #4 because NavigatorRAN/buzz has GitHub Issues disabled.
It is deferred until the production-shaped loopback Memory and RAG services,
signed mirror and golden cases, protected app configuration and Apple
allowlists, signed/notarised application, reviewed driver, disposable macOS
profile, and local packet-filter operator are available. The completed Phase 4
code gate must not be described as live operational proof.

The reviewed driver and operator procedure must:

1. Start from a signed/notarised application and a disposable macOS test
   profile with protected production-shaped configuration.
2. Record active interfaces, routes, listening sockets, process IDs, image
   digests, snapshot ID, Memory cursors, and the selected local model.
3. Install reversible packet-filter rules that deny internet and home-LAN
   egress for the app and local stack while retaining the required loopback
   paths. Record the rules and a timed rollback before applying them.
4. Restart Buzz, LM Studio, the local relay/data services, Memory, RAG, and the
   application. Do not infer offline operation from an already-running cache.
5. Prove local Memory read/write, mirrored RAG retrieval, allowlisted Apple
   input, five specialist calls, tool-free consolidation, terminal spool
   commit, and a complete or truthfully degraded brief.
6. Inspect packet-filter counters and process connection telemetry and prove
   there was no outbound OpenAI, LiteLLM, telemetry, webhook, updater, or
   home-LAN connection.
7. Re-enable only the local relay path, prove exact-event-ID publication, then
   restart the app and reload the same brief from signed history.
8. Remove the temporary rules through the recorded rollback and verify normal
   host connectivity.

Do not run this exercise against real OFFICIAL material until the relevant
Defence information-handling and host-security reviews are complete.

## Backup and clean-profile restore

The encrypted local workspace backup includes the command-brief SQLite store
and its schedule claims alongside the Buzz application state. Memory and RAG
retain their own protected state and provenance bundles and must be backed up
with the same evidence timestamp.

A controlled restore must use a clean macOS test profile and prove:

- backup authentication succeeds before any replacement;
- the exact command-brief database, schedule, claims, spool, and signed event
  IDs are restored;
- Memory restores its immutable journal, heads, conflicts, and cursors;
- RAG restores and revalidates the signed snapshot and retrieval dependencies;
- the prior brief decrypts for the restored owner and remains owner-only; and
- a fresh scheduled due check does not duplicate an already-claimed date.

Repository fixtures exercise clean-directory backup/restore behavior. They do
not claim that a production Mac profile, Memory corpus, or RAG bundle has been
restored.

## Resource measurement record

No live resource figures are asserted by repository tests. For each controlled
64 GB MacBook run, capture the following without inventing thresholds:

| Measurement | Before | Peak/end | Method and timestamp |
| --- | ---: | ---: | --- |
| Brief wall-clock duration | n/a | unmeasured | start/terminal audit timestamps |
| System memory and pressure | unmeasured | unmeasured | Activity Monitor or `memory_pressure` |
| LM Studio resident memory | unmeasured | unmeasured | per-process sample |
| RAG/Qdrant resident memory | unmeasured | unmeasured | per-process/container sample |
| Workspace storage | unmeasured | unmeasured | filesystem bytes before/after |
| RAG staging plus rollback storage | unmeasured | unmeasured | filesystem bytes |
| CPU/thermal observation | unmeasured | unmeasured | sustained sample and operator note |
| LM Studio/RAG health | unmeasured | unmeasured | native readiness and health timestamps |

Retain the raw command output or exported measurement file with the acceptance
record. Record observed values and limitations; do not convert one run into a
universal capacity claim.

## Known safety limits

- This is advisory decision support, not an accredited command, navigation, or
  ship-control system.
- `OFFICIAL` is the only Phase 4 brief classification and has no cloud
  fallback.
- Retrieved text is untrusted evidence and cannot alter system instructions,
  tool policy, classification, or egress.
- Source freshness and model quality remain operational risks even when
  cryptographic and structural validation passes.
- Apple privacy prompts and signed-app entitlements require controlled live
  testing; Xcode fixtures do not grant or prove production permissions.
- Repository tests do not prove a live air-gapped restart, zero egress,
  production signing/notarisation, real-corpus retrieval equivalence, clean
  production restore, or Defence accreditation.
