## Private instruction profiles (c66e)

`private-profiles.test.ts` adds actual offline CLI completed-input draft save,
reopen/edit/cancel/discard and stale-editor protection with an explicitly isolated
credential adapter whose invalid backing file makes accidental credential access
fail. The private NIP42/NIP44/NIP59 fixture exercises owner-to-self publication,
fresh **empty** local-journal reload with no host running, parent resolution,
wrong signer/relay and malformed revision rejection, exact authorized host Save,
stale CAS/tampered snapshot rejection, unchanged actual A on Save B, external
runner Restart receiving exact B, retained run history, and Stop.

The directly affected `catalog-tui.test.ts` installed case now saves a draft,
declines publication, explicitly resumes/publishes, selects the revision while
stopped, and proves exact installed native session instructions, signed reply and
Stop. It does not require instructions to be duplicated in each conversation turn.
Installed attribution: Buzz 0.5.23; binary hashes and exact logs in workspace
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/PROFILE_PRIVATE_C66_EVIDENCE/`.

Final executable strict passed; focused private+legacy profile tests **5/5**;
Nostr codec/auth fixtures previously **5/5** in the development focused set;
installed private TUI **1/1**. One default-concurrent whole-package run exited
naturally in 28.473s: **141 passed, 1 failed, 19 skipped** (installed opt-ins unset).
The unchanged admission-cancel wrong-authority control produced correct rejection,
Start and Stop receipts/state, but took 3233ms against its unchanged 2500ms bound.
It is the legacy transport/no-profile path; causality is not established. Full
suite is **not green**. No rerun, serialization, timeout increase or assertion
relaxation. Earlier new-test failures were fixture-path/model mistakes and an
incorrect expectation that native system instructions also appear in turn text;
all original logs are retained. No production relay/provider/credential operations.

## Current private transport validation

Setup/resume is configuration-only. `host-availability.test.ts` covers fresh npub
setup and retained resume with no HTTP or credential-helper calls, empty-host
private discovery and owner/placement negatives. `private-transport-denial.test.ts`
covers actual AUTH, REQ and availability publication rejection. Dedicated
membership evidence/signer tests and admission proofs were deleted deliberately.
`catalog-tui.test.ts` retains signed Start/reply/Stop and replay/foreign-owner
negatives; only its installed normal-private case needs renewed installed evidence.
Deletion validation at base `fcb82cf74b78dd1147bc9bc8316dc7ce97e17adf`:
Node 24.15.0 strict passed; one default-concurrent `node --test test/*.test.ts`
passed **140, failed 0, skipped 19** (installed opt-in unset). Only the directly
affected installed normal-private catalog Start/signed-reply/Stop fixture was
rerun with `BEEHIVE_REAL_BUZZ_ACP`: **1 passed, 0 failed, 0 skipped**, using the
existing explicit isolated credential and loopback relay/provider adapters.
Unchanged installed coverage reuses the preceding campaign; it is not new live
relay/provider proof. Two stale focused prompt/output assertions initially failed
and were corrected; failures remain in the external task evidence. No deadlines
or concurrency were relaxed. The dated results below are historical, not
validation of this deletion.

# Package validation

Activate Hermit from repo root, then enter `beehive`:

```sh
pnpm install --ignore-workspace --frozen-lockfile --ignore-scripts
npm run check
npm test
```

On Block-managed machines append
`--registry https://global.block-artifacts.com/artifactory/api/npm/square-npm/`
to install: the approved mirror is accessible anonymously while public npm is
policy-blocked. `pnpm-lock.yaml` is standalone and registry-agnostic. No borrowed
machine-path modules, alternate credential stores, or persistent config edits.
A fresh-store/fresh-node_modules frozen install fetched all seven packages;
strict TypeScript checking passed on Node 24.15.0 / pnpm 11.4.0. Keep
`--ignore-workspace` so installation does not expand to unrelated client packages.

## Client journal continuation

`intents.test.ts`: real WS loss before publication, original-envelope reopen,
post-host-commit receipt loss and automatic reconnect, retained terminal rejection,
unrelated/duplicate receipt fencing, abrupt child-process exit before network effects,
exact ciphertext/signature relay storage comparison, tampered file rejection, and
policy close 1008 blocking blind retries across reopen. Strict + installed-enabled
full suite passed 16/16; after status wording refinement strict + targeted TUI/client
5/5 passed, then final source strict/full default 15 pass + 1 opt-in skip. These fixtures do not establish production community admission, hostile
local storage/power loss or a polished manual recovery UX. See current CHECKPOINT.

## Policy recovery continuation 4341

`policy-tui.test.ts` drives actual Node TUI subprocesses over a fault proxy and the
real package relay/host with a fresh fixture runner. Before-publication Stop denial
remains UNKNOWN across reopen after repair; reconcile sends no blocked command;
numbered, informed retry sends the byte-identical envelope and resolves the original
Stop once. Repeated 1008 is bounded (one attempt per confirmation). A second journey
drops a committed receipt **before relay storage**, proving host outbox query rather
than merely cached relay history recovers it, with no extra lifecycle effect or
operation ID. Completed entries cannot be retried. Intentional Quit remains clean.
These are real client/relay/TUI subprocess seams, not a live model or production relay.
Independent d12ed7b4 crypto/receipt/perms and installed tool evidence remains reusable;
completed `INTENT_TUI_REVIEW_D12ED7B4.md` identifies U1, addressed by this delta.
The earlier pending/manual-recovery statements are historical, not a review wait.

## Automated production seams

- `slice.test.ts`: malformed/wrong-owner/tampered encryption envelopes; real
  loopback WS relay; durable encrypted records, operation fingerprints/revisions,
  wrong-agent denial, selected-next Save, external runner Start/Stop; two actual
  TUI subprocess sessions close/reopen while host survives; host restart retains
  assignment. F2 startup lock unwind and competing live lock exclusion; F3 deliberate
  Quit does not print UNKNOWN. F1 parent exit retains descendant ownership.
- `reconnect.test.ts`: drop the actual host management socket, publish Start
  while disconnected, verify relay-history recovery, drop after commit, then
  verify the same receipt/actual run and one revision/operation. Stop and
  intentional host/client close do not leak a reconnect. This is host recovery
  only; unsent controller intent durability and receipt ACK pruning remain open.
- `acp.test.ts`: external TypeScript ACP fixture under production supervisor,
  catalog provenance, model/session acknowledgement, completed text hash, wrong
  model/session, cancelled/rejected/timeout/malformed/flood cases. D3 completed
  response plus malformed line in one stdout write must fail evidence commit.
  Real encrypted management WS Start publishes fixture evidence; removing isolated
  auth directory does not prevent Save/Stop. Actual auth-info CLI does not login.
- `cancellation.test.ts`: real management WS and external delayed ACP prompt.
  D2 Stop at the in-flight Start's current revision and host close interrupt promptly
  rather than wait for the response; interrupted Start cannot commit running.
  Wrong-agent/revision/body Stop cannot interrupt. D3 malformed completion tail
  fails at the host seam as well as AgentSession.
- `admission-cancel.test.ts`: a Stop published beside a still-pending Start (live
  same-batch and through disconnected relay-history replay, in ACP and fixture
  host modes) cancels that admission before any run commits: the Start receipt is
  non-accepted/cancelled, the Stop is accepted at the same revision, the journal
  ends stopped with no actual run and no live-run inventory, and the verified stop
  releases the ownership lock. Controls keep the fence exact: a wrong-authority
  Stop in the same batch cannot cancel, a Stop published before its Start cannot
  cancel that later Start, replaying the already-terminal batch after another
  disconnect neither reruns nor recancels, and a fresh stale-revision Stop stays
  a conflict. This is the R1 replay-batch cancellation fix from the independent
  `TOOLS_RECONNECT_REVIEW_6F893F51.md` review.
- D1 `slice.test.ts` variants: resistant orphan descendants observe TERM but stay
  alive, then are removed by bounded live-anchor KILL escalation through both
  remote Stop and host close. `disconnect.test.ts` verifies parent IPC loss during
  runner life uses the same ownership-preserving teardown, including closed-IPC
  notification handling. Fixtures self-expire to bound failed regression runs.
- `conversation.test.ts`: persistent identity and exact external env, plus real
  management WS Start -> external runtime fixture -> escaping detached shim ->
  separately anchored harness, same-session completion and Stop absence checks.
- `broker.test.ts`: exact-model rejection/conflict, malformed coalesced completion,
  cancellation, reverse RPC, session/load, forbidden workspace/MCP provisioning,
  and teardown including TERM-resistant descendants. B1 now tests accepted and
  rejected optional config changes, preserving the original application error only
  after exact-model re-ack; failed re-ack and rejected model configs still fail.
  Tool-enabled foreign MCP provisioning remains rejected; thread arguments now belong to the CLI.
- `cancellation.test.ts` also exercises the conversation replacement path for
  admitted Stop, host close, non-authority attempts and malformed completions.
- `installed-conversation.test.ts`: opt-in with
  `BEEHIVE_REAL_BUZZ_ACP=/Applications/Buzz.app/Contents/MacOS/buzz-acp npm test`.
  Uses fresh fixture identities/dirs and an isolated NIP-42/REST/NIP-29 relay
  fixture. Verifies actual installed executable AUTH signatures, membership,
  owner mention dispatch, and exact-model same-conversation ACP completion through
  the broker. Its actual kind-9 subscription is respected. No provider is called.
  Three actual signed replies across two channels and a later prompt in the first
  thread are required through the installed sibling `buzz` CLI and fixed adapter.
  Verifies canonical hash/signature, same agent, channel/parent and explicit
  non-owner member p-tag; native channels list returns useful output, non-member
  mentions fail, and a non-owner inbound prompt is ignored; absence after Stop includes the MCP shim.
  Test code supplies the owner event and invokes the tool, never signs/publishes the
  expected agent response. This does not test a live LLM deciding to invoke tools.
- `reply-tool.test.ts`: production stdio adapter under detached childless shim;
  caller executable/env/signer/relay/attestation overrides and inactive calls cannot spawn a CLI;
  fixed argv/environment; success, cancellation, disconnect, failed startup and
  CLI error (returned without killing the session) all clean up the shim and any TERM-resistant in-group descendants.

Tests never open existing owner keys/profiles/cache, production services or native
client resources. Removal-public-slots checkpoint with installed opt-in
(`BEEHIVE_REAL_BUZZ_ACP=...`): strict TypeScript + DEFAULT concurrent full package
**45/45 pass, zero skips**, natural exit (see CHECKPOINT for the exact run). The installed fixture relay does not establish actual community admission,
current relay compatibility, provider/model attestation. Multi-thread delivery is isolated legacy-binary evidence only.

## Independent evidence and limits

Prior reviews are in workspace
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/EXECUTABLE_REVIEW_{94D1DA0E,6BD85947}.md`.
Prior c4f30176 broker independently passed strict/11 tests and installed replay in
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/BROKER_REVIEW_C4F30176.md`. Its B1 optional-config
finding is fixed with production regressions here. Independent `TOOL_REVIEW_023C9274.md` found no new blocking defect in the
predecessor MCP delta and independently confirmed B1 resolved (strict/full12/12,
original/extended B1 probes and signed installed replay). Independent
`TOOLS_RECONNECT_REVIEW_6F893F51.md` then covered the ordinary-CLI and reconnect
deltas (strict/full 14/14 installed plus its own probes; no new tool-authority
expansion) and is now consumed: its single R1 medium finding, a replayed
same-batch Start+Stop leaving the agent running, is fixed here with
`admission-cancel.test.ts` production regressions; its receipt-loss, backoff,
duplicate/fingerprint and CLI-option probes remain reusable evidence. The generic ACP fixture and signed/encrypted WS
management protocol are different seams; passing them does not prove current
Buzz relay auth/member/owner semantics.

No full repo `just ci`, multi-host Move, arbitrary escaped descendant containment,
strong OS sandbox, hostile journal, log-full/controller-reconnect/ACK, rollback, host SIGKILL
recovery or real provider proof. Genuine ownership loss retains quarantine/lock;
no stale numeric PID grants cleanup authority. See CHECKPOINT.md for exact next
executable action and the wired broker, ordinary fixed-executable CLI tool, and remaining product scope.

## Installed executable pins

- buzz-acp: `10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`
- buzz CLI: `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`

Both are read-only `/Applications/Buzz.app/Contents/MacOS/` binaries. Their digests
are not a claim that they were built from the source base. The MCP adapter supports
newline stdio and native CLI argv/stdin, with host-owned executable/identity/relay. Optional local attestation is forwarded using the normal CLI environment;
no live admission/delegation policy proof is claimed from the fixture relay.

## Assignment bootstrap continuation

`assignment.test.ts` starts three actual TS host subprocesses over one isolated
management relay: the same fresh agent key on source/standby plus a second identity
on a third distinct installation. Destination Start and cross-agent routing fail;
source Start/Stop/service restart retains assignment; absent source cannot authorize
standby. Actual TUI selects and shows both identities with exact host routing.
Deleting a running host's local setup does not block Stop, but subsequent Start
fails without reconstructing the key. Missing journals fail closed; explicit legacy
migration preserves revision/history and refuses replacement. This is assignment
bootstrap evidence, NOT Move or multiple agent slots on one host evidence.
All existing fixture setups now explicitly provision initial authority; none relies
on host startup self-assigning from key presence.

## Single-installation slot continuation

`slots.test.ts` uses the production host/client/relay and real external fixtures:
- ONE actual host subprocess exposes X and Y over exactly ONE management socket.
  Actual TUI subprocess selects numbered rows, saves/starts both, stops X, rejects
  ambiguous host-name selection and shows persistent identities. Y's journal stays
  byte-identical across X Stop, including revision, actual run and receipt.
- A reused operation ID on another slot remains independent. Wrong-agent/host
  operations cannot affect a run. Host socket loss replays both inventories/outbox;
  graceful process restart retains assignments and truthful stopped state. Same-key
  standby in another process never obtains Start authority.
- Concurrent delayed ACP Starts with the same operation ID reach transitioning
  independently. Stop X cancels X while Y is still pending; Y completes. Replays
  return distinct terminal receipts without mutating either journal.
- Explicit migration after real Start/Stop preserves the complete journal byte for
  byte. An uncertain first slot makes host close reject/retain the lock while the
  healthy sibling is nevertheless torn down and saved stopped.
- Actual management-client reopen/reconcile emits an inspect for EACH unresolved
  host/agent pair. Starting the real host later consumes initial relay history and
  resolves both retained intents. Slots are fully hydrated before dialing, so
  history coalesced with socket open cannot be silently dropped awaiting ready.

These are lifecycle/ACP fixture proofs, not simultaneous live provider/conversation
proof or Move acceptance. First new TUI harness iteration timed out because its
prompt parser assumed prompts ended stdout chunks; corrected to consume prompt
positions amid coalesced async status reports. A subsequent new reconciliation test
exposed the initial-history hydration gap above, fixed at the host connection owner.
No containment assertion, delay or existing test was weakened. The historical
reply-tool descendant-exit observation remains a separate open investigation.

## Recovered fixture Move and recovery labels

`move.test.ts` executes two real host subprocesses plus actual TUI and relay: Move X,
Y byte-identical, standby Start denied, wrong target revision preserves running source,
reverse successor and historical grant replay. Fault-injected real host/relay tests
lose grant and source receipt before receipt storage, replay outbox, remove target key
or change prepared setup, and verify assigned/stopped target and revoked source even
after source service restart. Delayed prepared replies exercise valid Stop/Save
cancellation, invalid Stop/Save inertness and late Start fencing. These are fixture
launch/admission tests, not provider readiness or destination conversation replies.

`policy-tui.test.ts` reopens two same-host/action/revision blocked operations and checks
both agent identities and operation IDs in numbered rows AND each retry confirmation;
declining sends neither. Existing immutable-envelope policy-retry tests remain.

Consumed `DESCENDANT_EXIT_EBC8B3CB.md`: exact historical cause remains unproved
(invalid fixture PID vs reuse); the group probes are evidence, not safe-Move approval.
`reply-tool.test.ts` now captures raw PID, actual probe errno, fixture spawn/error
provenance, and bounded per-PID process state on failure. It does not wait longer or
weaken ESRCH, and does not kill any PID obtained from observations. Full-suite green
results without recurrence do not classify the historical failure.

## Identity-bearing conversation Move continuation

`installed-move.test.ts` is installed-opt-in, using the same isolated NIP-42/NIP-29
fixture relay and deterministic ACP harness as installed-conversation. It provisions
ONE signer on two actual host subprocesses, source answer, remote Move, target answer
and a later owner turn. Verifies Schnorr/canonical events, channel/parent/non-owner
member recipient, native CLI read/rejected-mention behavior and post-grant broker
model/session/completion evidence. At grant delivery source journal is consumed/stopped
and source runner/descendant/shim probe ESRCH, before target can launch actual runtime.
Missing setup/unsupported selection preserve source; postgrant actual model rejection
cannot produce a target reply and leaves target assigned/stopped/source Start denied.
This is not live-provider or production admission testing.

`move.test.ts` additionally covers M1 from independent MOVE_REVIEW_97A9050A: target
changes setup across graceful restart after source consumption and before grant
receipt; replayed prepare retains immutable token, target accepts once but stays
stopped, duplicate grant is inert, source restart cannot resurrect authority.

The independent review supports the scoped exclusively supervised POSIX group barrier,
not arbitrary escaping processes. Its pinned full run had 28 passes/two failures
and timed out; do not relabel it green. Empty fixture PID bytes -> 0 was captured,
separate from the still-unclassified older observation. Fixture publishes a successfully
spawned positive PID atomically before cancellation tests; ESRCH remains mandatory.

## Explicit Restart continuation dad771

`restart.test.ts` exercises the real encrypted management relay and host admission:
unchanged identity/assignment, fresh actual run ID, duplicate replay with byte-identical
journal, missing setup preserving the old run without key restoration, same-batch Stop
retraction, ACP prerequisite model rejection preserving actual state, invalid Stop
inertness and authorized Stop interrupting a delayed probe with no late revival.
`slots.test.ts` now drives `restart` in the real TUI subprocess while both agents run,
asserting X's new run and Y's byte-identical receipt/revision/run journal.
This historical Restart-only increment had no profile-instruction acceptance; the
continuation below adds it. Installed-conversation Restart remains unclaimed.

## Behavior profiles and actual application (0aa71)

- `profiles.test.ts`: canonical hash/field rejection, missing parents, explicit
  concurrent branches and invalid references. Real relay/host ACP launch receives
  revision A; publish B and Save leaves actual A unchanged; async Restart preflight
  captures B while publication C and Save race it. Save conflicts, probe and actual
  harness both receive B. Stop cancels a subsequent delayed Restart without revival.
  Successful actual snapshots survive Restart and Stop in run history.
- Same file loses the Restart receipt BEFORE actual relay storage, reopens the real
  durable management client, recovers the host outbox and proves exactly two external
  launches (original Start + Restart), unchanged journal and no third launch.
- `slots.test.ts`: actual two-agent TUI creates/edits/publishes, selects A for X,
  Start X, Start Y default, applies B while X runs, checks unchanged X actual and Y
  complete journal, then Restart X with a new run/process and exact received B bytes.
- `move.test.ts`: actual two-host TUI Move preserves source-selected profile to the
  target despite its default configuration, records received target instructions,
  and preserves sibling Y. The first new assertion initially ran before the fixture
  recorded startup (spawn is not script readiness); it now awaits that explicit
  external record before asserting bytes. No production readiness wait was weakened.
- `installed-conversation.test.ts`: read-only installed buzz-acp/CLI and existing
  deterministic ACP harness receive an explicit profile marker both in harness env
  and in the runtime-generated actual ACP prompt. Three native signed replies still
  obey owner/member/thread/model fences. This is not an installed Restart journey
  or live model/provider proof; real Restart/profile application uses external ACP
  and lifecycle fixtures above.

Strict and installed-enabled default concurrent full-package results and exact source
attribution are in CHECKPOINT. No test script, serial mode or containment assertion
changes. Restart and profile deltas still need independent immutable review together.

Final candidate iteration history: first default concurrent full 38/38 (29.78s).
After run-history additions, default full 37/38 (29.53s): unchanged broker assertion
read an existing but empty `reverse-rpc` marker. The fixture had open/truncate/write
publication while the reader gated on existence. That marker now uses atomic
write/rename; broker logic, timeout and assertions unchanged. Targeted broker 1/1;
final exact executable strict + default installed-enabled full 38/38 (29.50s), natural
exit. This captured failure does not classify the older unrecorded broker failure.

## Named launch configuration continuation

`configurations.test.ts` drives the actual local setup CLI subprocess (not a helper)
through fixture setup and two deliberate key creations sharing one harness. It checks
0600 files/0700 installation directory and that stdout contains none of the fixture
owner/agent secrets. Then actual TUI subprocesses create Alternative, Save a different
locally allowed workspace, select default/Alternative while X runs, and Restart X.
The old actual run and Y's complete journal stay unchanged; the new actual snapshot
is Alternative@3/exact workspace under the same X identity. Target Destination@2 is
created/saved remotely while standby Start stays denied; stale candidate Move preserves
source, exact candidate Move starts the destination. The setup manifest is byte-identical
through all remote configuration/lifecycle operations: no key/setup copy or regeneration.
The helper-level test covers rename/removal/name bounds and revision non-reuse.

`move.test.ts` additionally drops a consumed grant, accepts a remote target candidate
Save, then delivers the retained grant. Assignment must advance but target stays stopped
with its newer selected-next, source remains revoked, and duplicate/restarted-source
replays cannot resurrect it. A changed target revision cannot silently strand a consumed
grant. Reverse-Move acceptance now uses the exact observed source selected-next rather
than a hand-authored selection omitting its newly materialized configuration revision.
Existing profile/async-Restart Save races and independent per-slot cancellation tests
continue exercising the same serialized Save/CAS path. No timeout/containment assertion
or production broker was weakened. Exact final suite result is in CHECKPOINT.

`remove-key.test.ts` covers deliberate local key removal end to end with actual CLI
subprocesses. A declined confirmation changes nothing; `remove-agent-key` then deletes
only the local secret copy (manifest `secret: null`, 0600/0700 preserved, secret bytes
gone and never printed, journal/sibling byte-identical). Reopen advertises the
missing-key public slot; Start/Restart reject before any spawn (fixture runner line
count proves no process) and no lifecycle/recovery path recreates the key; Stop/Save
stay credential-neutral; management-socket loss replays truthful receipts/inventory.
Active-host removal refuses at the installation lock without deleting it, and a
non-stopped/unknown/already-removed/malformed identity fails closed; legacy key-less
installations reject remove/migrate/conversation-setup/host startup; public-only
slots reject conversation setup. Two real host subprocesses prove Move semantics:
a Move to a missing-key destination fails preflight and preserves the running source
(manifest unchanged), and a public-only source still consumes its retained
management authority so the destination launches with its own local key (source
manifest byte-identical, consumed source denied Start).

## Named Move F1/F2 regression combination

`named-move.test.ts` adapts the independent review's original real relay/host/TUI probe:
profiled source to named/default destination, no manual property-order workaround,
consistent named grant/inventory/selected/actual, preserved prior definition and history,
reselect + Restart, both host reopens, once-only duplicate grants and reordered wire
inputs. It also retains the original nine rejection controls and actual-referenced removal.
`named-move-recovery.test.ts` adds profiled/named versions of dropped-grant target restart
and later standby Save: reserved @2 cannot collide with later same-name @3, stale CAS
rejects, original preparations persist and source never resurrects. `handoff-selection`
checks strict semantic grant equality, legacy identity hashes, extra/missing/different
fields, unknown materialization and ordered-array negatives. Original `move.test.ts`
M1/replay/restart/later-Save/cancellation cases remain; reverse named Move now explicitly
expects reservation revision 4 then Start revision 5. The first legacy implicit-default
Move now asserts default@2, selected/inventory/actual equality, and Start revision 3;
later Save and cancelled-preparation retry use the explicitly reserved CAS revision.
No timeout/assertion relaxation.

The concurrent slots TUI driver observes committed revisions through actual `show`
output before subsequent lifecycle commands, rather than assuming its private host
journal gate means the separate TUI has consumed inventory. Gate failures close the
owned UI and propagate; existing assertions and observation/test deadlines are unchanged.
Retained direct diagnostic captured a stale-observation Restart revision-conflict;
other older timeout observations lacking that trace remain unclassified.

## K1 and explicit local key import continuation

`remove-key.test.ts` adds actual removal CLI missing-assignment/mismatched-genesis
and malformed owner/holder/root/predecessor/grant/operation negatives, retaining
manifest and journal bytes. Stopped, standby and consumed-source positive controls
use the same read-only host admission loader. Existing real Move tests remain.

The import test exercises actual CLI confirmation, wrong key and malformed hidden
input, lock and corrupt-authority refusal, exact secret-only manifest change,
0600/0700 permissions and unchanged sibling/history. On macOS it also runs the
actual wizard inside `/usr/bin/script`'s PTY and asserts no key echo. The fixed
`cat | script` test adapter gives macOS a real pipe instead of Node socketpair
stdin; it does not inject key argv. Other platforms exercise the subprocess but
not that macOS PTY. A real host and loopback relay then deny imported standby Start
without spawning or remotely mutating the manifest. No provider calls are involved.

## Standby startup diagnostic driver (d8fe)

`host-driver.ts` replaces discarded child stderr in the slots TUI workflow with
bounded 8 KiB stdout/stderr tails and reports spawn error, PID, exit code and signal
on early exit or readiness failure. Its original 400 × 20 ms observation bound is
unchanged. `host-driver.test.ts` falsifies lost stderr/exit status and unbounded
retention using a 20 KiB failure stream, and verifies spawn-error propagation.
Use this driver only with fresh fixture identities, not secret-bearing owner
processes. Focused driver/slots 6/6 and final DEFAULT concurrent installed-enabled
53/53 pass. The prior 50/51 standby timeout did not recur and remains unclassified;
this is diagnostic coverage, not evidence of a production startup fix.


## Explicit reusable host-local bindings (fd5123)

`bindings.test.ts` provisions immutable bindings through the lock-protected local API,
then uses real host subprocesses, relay and TUI to select B in Alternative while X
runs A and Y remains byte-identical. Explicit Restart starts the different B runner
with received selected-profile bytes, same agent identity and immutable A history.
The target config selects its independently fingerprinted local B while standby has
no Start authority; Move preserves source behavior. Remote Save/select/replay never
rewrite the key manifest. Unknown/mismatched binding references and incompatible
models reject. Lock test denies local mutation and a real competing service Start
under the same 0700 atomic mkdir lock; manifest remains 0600 and authority unchanged.
`binding-recovery.test.ts` retains the original named Move recovery tests unchanged
and adds explicit-B variants: changed prepared definition across target restart,
immutable preparation/grant replay, stale explicit Start rejection, and late standby
Save/CAS leaves target assigned stopped without overwriting newer selected-next.

This is deterministic external-runner/profile evidence, not Goose, live provider,
production protocol or integrated binding CRUD/wizard acceptance. Existing K1 and
F1/F2 independent reviews stay closed; this new delta has author self-review only.
Exact full-suite results and iteration failures are in CHECKPOINT and workspace logs.

## Initial transport and local wizard probes (7b70)

`startup-transport.test.ts` owns a fresh loopback HTTP listener and deliberately
withholds WebSocket upgrade without blocking the server event loop. The unchanged
production 2000ms handshake timeout rejects host startup, releases its installation
lock, preserves the journal and does not auto-retry. A subsequent explicit host
start succeeds. Other probes close an outstanding initial client handshake and
return HTTP 403 with recovery enabled: ready rejects, no silent retry/recovery.
Established recovery remains separately covered by `reconnect.test.ts`.
These probes explain the failure path, not the cause of the historical slots timeout.

`bindings.test.ts` now provisions B through the actual local wizard before its
remote A/B journey. Additional actual CLI new/reuse/hidden-restore acceptance
asserts unchanged journals, independent keys, chosen binding and no secret output.
API stale fingerprint checks run under the same atomic lock as host start.

## Goose native ACP and local entry (768f)

`goose.test.ts` drives actual CLI initial Goose NEW/reuse and hidden standby import;
on macOS the initial import runs in a real script PTY. It verifies public-genesis
matching, no key echo, modes, standby Start denial, actual native session/profile/
model Restart through real host/relay, unchanged identity/Y/manifest/history,
unknown/incompatible/stale selection refusal and wrong-model preflight preserving
actual. Removing the dedicated service HOME still permits valid Stop. Native model
codec negatives bind production `gooseModels`, not a test-only adapter helper.

`goose-binding.test.ts` uses the actual local wizard to add an immutable Goose
binding to a fixture installation, then actual remote TUI chooses that binding and
compatible second model while the fixture actual remains unchanged. Explicit Restart
executes a source-shaped native ACP session (not the fixture lifecycle runner), old
actual survives in history, Y and manifest remain byte-identical. Advertised models
must track the selected binding. Generated executable is an owner-only TypeScript
fixture with exact `acp` argv, never an installed Goose executable.

`installed-conversation.test.ts` now covers both Buzz Agent and Goose-shaped
protocols under actual installed buzz-acp + Buzz CLI. Goose fixture rejects unstable
set_model, reports native configOptions and implements the pinned Goose system-prompt
extension. Three real CLI signed replies, same signer, two channels/later turn,
profile/native-model evidence, admission/tool recipient fences and complete owned
Stop are asserted. It enters the broker directly; wizard-to-installed-conversation
continuity and live Goose/provider authentication are NOT established. All source
shape assertions and installed evidence remain separately attributed. Full default
concurrent package results/failures are recorded in CHECKPOINT, not replaced by these
focused passes. No transport deadline, test default, or assertion was weakened.

## Nonblocking durable development relay

`relay-persistence.test.ts` binds the actual async atomic writer at controlled
write/rename/directory boundaries (no timing sleeps). While a write is suspended,
a second real HTTP WebSocket upgrade and ping/pong complete; both readers see only
committed history. Concurrent publications and duplicate retain exact signed order,
reconnect and reopened relay replay the identical JSON snapshot, 0600/0700 and temp
cleanup hold. Write/rename failures neither echo nor poison a queued sibling.
Close drains pending durability even after publisher disconnect. A post-rename
fsync failure is uncertain, not a guaranteed rejection: no echo, subsequent commits
and readers fenced, close callback receives the error. The unacknowledged replacement
may be recovered on reopen. This is file/directory fsync evidence, not power-loss
simulation. The fixture-only optional phase hook is not exposed through CLI/env.
Full-history JSON encoding/replay and the 10,000-entry limit remain; this change is
not append-log/compaction or unbounded history support. Existing synchronous storage
helpers for host journals, keys and configuration are intentionally untouched.

Async durability no longer blocks unrelated test polling: a host's private journal
commit is not proof the TUI has consumed its subsequent inventory, and submitting a
profile is not proof its numbered row exists. Config fixture now queries actual
`show` revisions (captured stale Save/revision-conflict); slots queries actual
`profiles` for exact instructions before numbered actions. Original outer deadlines,
CAS assertions and operation counts remain. Admission failure assertions retain
bounded fixture storage phases, actual receipts and durable receive order rather
than assigning elapsed-time failure to storage without evidence.

## Disconnect classification continuation 0346

`disconnect.test.ts` already awaited `resistant.ready`, published after the
fixture's SIGTERM handler installation, not merely OS spawn readiness. New capped
fresh-fixture lifecycle trace captures handler installation, TERM callback entry,
marker write/error and 15s self-expiry; failure output retains that trace before
cleanup. Same positive descendant/cwd and unchanged TERM/group-absence assertions
pass in one focused probe and final concurrent full run. This is diagnostic
coverage, **not a historical ENOENT fix** or production containment change.
Actual Goose setup CLI now asserts an honest ACP-probe-only completion warning,
including explicit local conversation transport and unverified admission/login.
Normal conversation wizard integration and installed selected-B Restart remain open.
Final strict passed; full default installed-enabled 69/70, natural 32.943s, with
unchanged policy-tui revision-1 expectation versus revision-0 output failing.
No repeat, timeout change, serial mask or repository-wide CI claim; see CHECKPOINT.

## Normal wizard A661

`normal-conversation.test.ts` reuses the independent D22 installed journey with no
manual conversation attachment. Initial normal-default and selected diagnostic B
conversion both drive actual CLI/TUI, production host(), installed buzz-acp/CLI and
native-shaped Goose fixture. Each verifies three signed same-agent replies over two
channels/later turn, then fresh installed Restart/session/exact model/profile and a
fourth signed reply. Selected-B journey retains A history and immutable A/B definitions;
Y and key manifest remain byte-identical through remote Save/Start/Restart/Stop.
Wrong-model preflight preserves actual; completed rejected Restart and Stop replays
cannot revive it. Goose profile is observed at its native system-prompt-extension
output, not assumed to be duplicated in each turn prompt.

`conversion.test.ts` covers source fingerprint, active-state and atomic lock refusal,
common-authority mismatch, duplicate binding rejection, declined actual wizard,
manifest-only activation/reopen and normal hidden standby import (macOS script PTY,
no secret echo, retained assignment, 0600). Initial/imported executable paths here
are deliberately metadata-only fixtures; no provider/login or runtime spawn claimed.
Existing Goose/assignment tests retain actual standby Start denial and secure-input
negatives. The normal installed journeys run only with the existing installed opt-in.

## CLI lifecycle recovery 5360

`cli-shutdown.test.ts` launches the actual relay CLI. Test-only module loading
injects the existing directory persistence boundary: wire1011 and possible
replacement must lead to uncertainty stderr and SIGINT exit1, versus clean exit0.
`startup-transport.test.ts` adds actual host CLI SIGINT/SIGTERM while upgrade is
withheld, clean lock/journal/restart checks, quarantine retention and double close.
`policy-tui.test.ts` now waits for relevant PUBLIC running rev1 inventory before
spawning Stop TUI; private journal rev1 is not asynchronous relay visibility.
Original unknown/byte-identical retry/accepted rev2 assertions remain unchanged.
`normal-conversation.test.ts` now starts actual host CLI subprocesses from the
wizard-produced installations instead of in-process host construction. Initial
normal and selected-B conversion still require four signed installed CLI replies
including Restart, same identity, exact model/profile and unchanged sibling/history.
Strict + one default concurrent installed-enabled full run: 78/78, natural 44.186s.
This closes scoped service-subprocess continuity, not real Goose/provider or
production kind40002/service deployment proof. See CHECKPOINT for historical failures.

## Claude source-grounded subset (6f8d)

`claude.test.ts`: actual owned source-shaped ACP session; dedicated closed env,
owner-only local key file, missing/unsafe prerequisites, native model/profile,
wrong model/package/auth rejection/drift and owned descendant absence. The vendor
CLI is a Node placeholder and the adapter a TS fixture, not real Claude.
`normal-conversation.test.ts` now retains both Goose cases and adds both Claude
initial/selected-B cases through the same actual wizard/host CLI/TUI + installed
buzz-acp/Buzz CLI four-signed-reply path. Private env is checked inside the fixture,
private inputs absent from public inventory/output, wrong model/native/auth failures
preserve actual; missing credential does not block Stop. No Codex acceptance is
claimed. See CLAUDE_SOURCE_6F8D.md for precise source contract and limitations.

## Codex conditional native path (29c7)

`codex.test.ts` exercises the production owned ACP boundary with explicit JSON
model overrides, dedicated HOME/CODEX_HOME, key-file prerequisite and exact native
protocol2/profile/model report, before-prompt refusal and later drift. Vendor CLI
is a Node placeholder, not Codex. `normal-conversation.test.ts` includes both Codex
initial and selected-B conversion journeys with four signed installed CLI replies,
Restart and preserved identity/history/Y, wrong protocol/model/auth/native negatives,
actual local key removal/service reopen, no missing-key spawn/recreation and neutral
Stop. These use the existing installed opt-in and unchanged default concurrent runner.
Source contract and limitations: `CODEX_SOURCE_29C7.md`; exact logs/hashes are in the
workspace evidence directory named there. No provider or current relay approval.

## Binding replacement and retirement (1bc0)

`bindings.test.ts` reuses the actual local wizard and remote TUI/Restart journey:
replacement creates B and retires its predecessor atomically without changing A,
identity, selected-next, sibling or retained history. Separate retirement probes
exercise declined/stale confirmation, stopped-state and competing startup-lock
refusal, immutable fingerprint/0600 preservation, corrupt-definition rejection,
retired selected host reopen, no runner spawn for Start/Restart/Save/select, and
credential-neutral Stop/configuration rename/removal. The wizard displays affected
agent/configuration choices and requires confirmation; it never selects or starts.
`binding-recovery.test.ts` adds retirement to its delayed consumed-grant/reopen
harness: original preparation and definitions survive, target accepts authority but
stays stopped, replay is inert and source restart cannot revive consumed authority.
`normal-conversation.test.ts` reuses all three selected-B normal journeys with an
additional actual local replacement before explicit remote selection: installed
buzz-acp/CLI still produce four signed replies, including explicit Restart with
same identity/profile and unchanged sibling/history. Provider adapters remain
source-shaped TypeScript fixtures, not live Goose/Claude/Codex.

## Owner-public v3 immutable bindings (7fdd)

`v3-bindings.test.ts` binds the existing CLI wizard, public preview/hash/lock,
replacement/retirement, neutral conversion, actual A -> selected-next B -> Restart,
removed-key Save/Stop and failed-preflight survival. A deliberately invalid JSON
synthetic credential backend proves neutral wizard actions never query it. The
host errno test checks receipt sanitization, not inventory workspace suppression.
`catalog-tui.test.ts` adds v3 conversion through the actual local wizard and private
owner TUI before the installed ACP/Buzz CLI signed-reply journey. Other provider
journeys are unchanged; no provider parity expansion, OS-store or live enrollment
proof. Workspace V3_BINDINGS_7FDD records exact execution/failures.

## Owner configuration and ordinary join (3a946)

Normal setup now uses OWNER NPUB/hex + retained relay configuration, not approval
exchange. `host-availability.test.ts` drives actual fresh CLI configuration,
nonmember cancellation, policy-required refusal, issued-link claim confirmation,
member resume, zero-agent host/private owner discovery, wrong-owner/absent-agent
Start rejection and missing-key refusal. Both credential boundaries are explicitly
isolated. `launcher.test.ts` covers npub/default paths and retained resume;
`enrollment-input.test.ts` retains historical explicit exchange compatibility.
`catalog-tui.test.ts` now runs normal setup before approved and unapproved host
Start/Stop, including installed Buzz signed-reply and v3-conversion scenarios.

Validation: one default-concurrent installed-enabled whole-package run completed
naturally in 311.190s: **167 passed, 0 failed, 0 skipped**, including all 19 installed
opt-in cases (`BEEHIVE_REAL_BUZZ_ACP=/Applications/Buzz.app/Contents/MacOS/buzz-acp`).
This run predates the subsequently discovered NIP-98 nonce correction: same-second
requests previously had identical event IDs and native replay protection refused
rechecks. `membership-signer.test.ts` binds the production signer to unique signed
IDs and a replay-enforcing loopback fixture; the nonce matches native Buzz CLI's
existing contract, not a new relay API. Final source strict + focused
`membership-signer`, `direct-membership`, `host-availability`, `catalog-tui`,
`nostr-transport`, `launcher`, `enrollment-input`: **30 passed, 0 failed, 0 skipped**,
installed opt-in enabled. The whole suite was not repeated after that causal fix.
Original failing replay assertion and stale setup-check-count assertions are
preserved in the external work log evidence; no retry-to-green or serial masking.

These are synthetic local credential/relay/provider fixtures using installed
binaries, not live OS credential, community admission, policy acceptance or vendor
readiness evidence. Authority foundation 91d2246 received a scoped independent
no-blocker review; it must not be represented as review of this later setup slice.
