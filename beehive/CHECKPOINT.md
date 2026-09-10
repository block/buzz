# Beehive executable checkpoint

Worktree `/Users/loganj/.buzz/REPOS/beehive-cbfd9440`, branch `beehive/cbfd9440`.
Base `051c3a270be9c73da9ab06700bcab7d5552fceaa`; continuation starts at
`023c9274767ef50fa0f5b37ef1883336f8be59fd`. Candidate is the commit containing
this checkpoint (`git rev-parse HEAD`). No merge/release.

## Goose / integrated local entry 768f — published partial checkpoint

Final strict passes. Exactly one final **FULL DEFAULT concurrent installed-enabled
package run completed 63/65, two failures, zero skips/cancellations, natural 35.904s**.
All four added Goose tests pass in the full run. The two failures are unchanged
owners: slots.test.ts:131 standby host PID 96255 exits 1 with `Opening handshake has
timed out`; admission-cancel.test.ts:126 controls exceed the unchanged <2500ms bound.
No rerun, serial mode, assertion/deadline relaxation, or transport patch. The slots
symptom recurs; this uninstrumented candidate run does not independently establish
timing cause. Investigator's separate measured baseline finding is retained below;
admission cause remains unclassified. **Full-suite/engineering acceptance OPEN.**

Self-review covered native model/no-fallback and subsequent update checks, provider
env isolation, selected-binding inventory, hidden-reader handoff, exact public key/
owner/genesis checks, lock/fingerprint ownership and unchanged manifest/history.
No dependency, old native/client, owner credential, provider login or production
changes. Node24.15.0/pnpm11.4.0, installed hashes freshly match retained pins:
buzz-acp `10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`,
buzz `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`.
Exact executable source hashes are in `GOOSE_768F/final-source.sha256`, full result
in `full-installed-default.log`, prior failures and focused logs retained beside it.
Candidate is the commit containing this checkpoint; terminal publication records
exact remote head/clean-tree and Logan Johnson author/committer+DCO verification.
Next: review/repair the measured relay persistence owner without weakening durability
or timing gates, then final concurrent validation; close the explicitly separate
wizard-to-installed conversation integration gap before claiming that full journey.


Continuation from clean published `3b3ba27aa1436a46a5777f700af15bc4d90fb621`.
Actual implementation, not transport instrumentation. Source boundary remains
pinned Buzz `051c3a270be9c73da9ab06700bcab7d5552fceaa` (git-show, not checkout HEAD):
Desktop discovery/catalog.rs Goose entry disables unstable ACP model switching,
uses `GOOSE_PROVIDER`, `GOOSE_MODEL`, `GOOSE_MODE=auto` and Goose-owned config under
`~/.config/goose/config.yaml`; buzz-acp config.rs:792 supplies `acp`; acp.rs:707–719
supplies `_goose/unstable/session/system-prompt/set`, and :2150–2185 describes
native category=model configOptions. Goose itself is NOT installed/executed by
this work, and no upstream Goose-release compatibility or provider login is claimed.

Implemented a narrow spawn-fixed Goose local binding at existing host preparation,
AgentSession and conversation broker owners. Separate service HOME/provider context,
no Buzz Agent env/cache inheritance, no remote executable/env/credentials. Native
session model currentValue plus advertised options must match the selected exact
model. Missing/mismatched/ambiguous evidence rejects, never fallback to invented
session/set_model support. Stable model-setting replies must re-establish native
evidence; later model drift rejects. Profile uses the source-shaped Goose system
prompt method in direct probes and the installed runtime's own implementation in
conversation mode. Settings require selected-next Save and explicit fresh Restart.
Inventory models/workspaces now follow selected binding, not original slot default.

`setup <existing-host>` enters the local wizard without allowing owner replacement;
new setup offers Goose, independent NEW identity or hidden exact standby import
against a public genesis. Existing wizard adds immutable Goose binding (source
fingerprint rechecked under host.lock), existing-key reuse, new identities, hidden
unknown standby import on a selected binding, and separately explicit retained
public-only restoration. Common host/control/relay authority stays fixed. Import
never grants Start, including initial hidden import. No key/session/workspace transfer.
Initial host startup signal ownership is unchanged: handlers still start after ready;
signal-during-startup cleanup remains unestablished, not silently claimed fixed.

Focused evidence: strict passes; UX/ACP/bindings **8/8**, including actual CLI initial
Goose new/reuse/hidden standby (macOS PTY, no key echo, 0600/0700), real host/relay
profile/model/fresh-session Restart, wrong-model preflight preserving actual, neutral
Stop without service HOME, stable identity/Y/manifest/history. Actual CLI add-Goose
binding -> actual remote TUI model selection -> explicit Restart executes the native
ACP consumer, preserving old fixture run/history and Y. Broker/key/installed **9/9**:
real installed buzz-acp + installed Buzz CLI, Goose-shaped TypeScript ACP fixture,
3 canonical signed same-agent replies, 2 channels, native model config and received
profile, owner/member/thread fences, verified owned Stop/descendant absence. This
installed proof enters ConversationSession directly, not the entire wizard-to-installed
conversation path; the selected-setup journey separately exercises direct ACP.
Do not combine those into a claim of an uninterrupted installed provider journey.

Retained iteration failures: initial strict env-union typing (fixed), installed
Goose-shaped fixture omitted the native system-prompt method and exited; source
inspection + bounded fixture method trace identified it, fixture corrected. New
profile test initially did not publish its immutable revision; fixed test input.
No deadline or assertions weakened. Logs in
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/GOOSE_768F/` (workspace absolute prefix
`/Users/loganj/.buzz/`). Source and installed-binary attribution remain separate.

Consumed investigator 521307cf terminal a7fb3981: one immutable full capture 60/61;
slots standby handshake recurred with synchronous relay persistence starving accept;
admission controls passed, prior failure still unclassified. No investigator edits
copied; no transport change or further diagnostic/full rerun-to-green here. Final
strict/default concurrent installed-enabled gate follows this checkpoint after capture
completion. Engineering acceptance remains open until its exact result is recorded.
Full Claude/Codex/provider/preset/custom/mesh/compute parity, live Goose/provider auth,
production kind-40002 admission, destructive binding editing/deletion and one continuous
wizard -> installed conversation journey remain. Previous K1/F1/F2 reviews stay closed.

## Startup / local wizard 7b70 checkpoint

Continuation from clean published `6a28b390db2caca6d45699f91709a2d9a277b053`.
Evidence directory: `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/STARTUP_7B70/`.
Predecessor BINDINGS_FD5123 logs are untouched. Strict + initial transport/reconnect
5/5 and local wizard/binding 3/3 focused passed before final gate.

Proved: configured `ws` handshake timeout is 2000ms. Its initial error rejects
`client.ready`; host catch closes the client and removes the owned lock; CLI prints
that error and exits 1. Initial failure is deliberately NOT established recovery.
The slots child driver awaits asynchronous 20ms polling; it does not synchronously
wait on its relay process. Relay awaits listening before the child is launched.
Relay verification/persistence/history delivery are synchronous work on the test
process event loop, but the captured run has no timing/upgrade trace proving that
work delayed this handshake. Contention/readiness/event-loop cause remains
**unclassified**, as do unrelated historical timeouts. No production retry,
timeout inflation, serial default or rerun-to-green was introduced. Deterministic
withheld-upgrade, cancellation and HTTP denial probes bind the actual client/host.
Transport-level close aborts startup; CLI signals are still installed only after
host readiness, so signal-during-startup lock cleanup is not claimed by this test.

Product increment: `local-setup` lists/reuses bindings/identities, adds compatible
immutable binding IDs, creates independent keys on a selected binding, or restores
an exact retained key through the hidden reader. Mutation APIs revalidate draft
source fingerprints under atomic host.lock; no key goes into binding data. New
binding and subsequent agent creation are two separately confirmed actions, not a
torn multi-write wizard transaction. Real CLI setup -> binding B -> remote A/B
Restart/Move test preserves existing identity/profile/history/sibling guarantees.
No edit/delete, second real harness, provider form, live auth, current production
kind-40002 or full-product acceptance. Initial setup/unknown standby key import
still use their existing separate flows. K1/F1/F2 closed prior reviews remain
reusable, not reopened or claimed as independent review of this increment.

Final strict passes. Exactly one final FULL DEFAULT concurrent installed-enabled
run naturally completed **59/61, two failures, no skips/cancellations, 35.513s**.
All new startup/wizard probes and the real CLI->remote A/B journey passed. Failures:
(1) slots.test.ts:131 standby host PID 88992 exited 1 before online, bounded stderr
`Opening handshake has timed out`; underlying cause still unclassified, NOT fixed;
(2) admission-cancel.test.ts:126 `Controls must resolve without waiting on a run`
asserted elapsed time <2500ms; this run exceeded that bound, cause unclassified.
Neither failure was rerun, suppressed or attributed to contention without evidence.
Full-suite acceptance remains OPEN; natural full log and focused logs are retained.
Self-review checked one-action writes, private key exclusion, hidden reader ownership,
stale-input lock checks, unchanged history and no new provider claims. Installed
binaries were read-only/hashed separately; source hashes saved beside logs. No
repo-wide CI, production/provider activity, dependency or persistent git config change.

One next diagnostic: instrument the actual slots relay's upgrade/event-loop timing
in a bounded fixture-owned trace to distinguish parent work from transport delay;
do not change retry/deadline policy on this evidence. Product remaining: initial
new/standby-import unification and startup-signal cleanup ownership, followed by
properly grounded real harness/provider parity.

## Binding fd5123 — published-candidate partial handoff

Executable candidate is the commit containing this section; remote verification is
reported in the terminal handoff. Recovered baseline was exact published
`1e6ead550e0bbf6cf8f817d07732a9b88fc84160`. Final strict passes. Final FULL DEFAULT
concurrent installed-enabled package: **56/57, one failure, zero skips/cancellations,
natural exit 34.240s**. All four new binding tests passed in that full run. Failure
is unchanged slots standby startup at slots.test.ts:131: host PID 75606 exited 1,
stdout empty, stderr `Opening handshake has timed out`. This newly captured
occurrence is a management transport opening-handshake timeout; underlying cause
and relation to historical unclassified timeouts are not established. **Full-suite
acceptance remains open.** No serial masking, timeout inflation or rerun-to-green.
Earlier full 56/57 (33.129s) failed the new TUI driver observation gate, preserved
separately; revision-observation correction is documented below.

Final run evidence: X `35de98eee5ce7b9be24e5d11d96cb76365f118a396d487647979ee02073dc9bd`,
Y `eeb07ddb3898d66c66f4031f9c7fb70e01297eed74ecf0bd24300a62cea85f2f`;
A fingerprint `b0e30b92cfa4a24deea32d53252d5612b65e5f483f28a87dd14970752147ae92`,
source B `77afae3f04095f15e778d9bd6a4c8c882663f49600eec0e0dd590db0dbdff80c`,
target B `97aac17667d55c62d182fcb6ae67f38432170c9bb7fdd9996dbdb8c0465da75a`.
B received exact profile marker/revision
`fa46ad839d945ac1016e3f75d18eb1146aa5c12717f3111c96e514bb1c013717`.
Named Alternative@4 applies only on explicit Restart; actual A and history retain
A, same identity, Y whole journal byte-identical, both host-local manifests retain
independent keys/authority. Target uses its different B fingerprint and preserved
source behavior. Replay and explicit stale-definition refusal remain tested.

Self-reviewed resolver use at Save/cancellation/preflight/actual launch; immutable
binding API owns the same atomic mkdir lock as Start. No parallel controller/runtime,
remote executable/env injection or key rewrite. K1/F1/F2 independent reviews are
reused unchanged, not reopened and not claimed as independent review of this delta.
Installed runtime/CLI hashes separately rechecked and match retained pins; source
hashes, both failures and focused/full logs are in BINDINGS_FD5123. No dependency or
global config changes. Logan Johnson author/committer + DCO, no crypto signer.

**Next:** investigate the freshly captured management opening-handshake failure under
the default concurrent installed-enabled shape without widening deadlines, then
repeat final strict/full on any corrected candidate. Integrated setup wizard,
full binding CRUD, Goose/provider path and full parity remain. This is actual
fixture A/B execution, not full-product/live-provider/current production kind-40002,
normal service-user OAuth, repo-wide CI, merge/release or production acceptance.

## Binding fd5123 — concurrent validation correction

First strict passed; FULL DEFAULT installed-enabled naturally exited **56/57**, one
new A/B TUI driver gate failure (33.129s). Original failure log lacks the step/output,
so its specific missed observation remains unclassified. The new driver copied an
80ms sleep from configurations.test.ts as a supposed receipt→inventory fence. This
is not a valid observation barrier; replaced it with existing slots-test actual TUI
`show` revision observation before revision-sensitive commands. Failure now retains
step and bounded TUI output. No test timeout inflation, removed assertion, changed
product admission or serial run. Revised strict and focused TUI/lock **2/2** pass
(10.570s). Final DEFAULT full run follows on this changed test candidate; original
failure remains in `full-installed-default.log`, not overwritten or called diagnosed.

## Binding fd5123 — pre-final-validation checkpoint

Focused new tests pass **4/4** (9.445s): actual source/target host subprocesses,
relay and named TUI A→B Restart with selected profile received by distinct runner,
unchanged X identity/Y whole journal/A history, different target-local B fingerprint,
manifest-neutral Save replay, unknown/stale binding rejection, standby B configuration,
profile-preserving Move. Explicit-B recovery adds consumed-grant replay after changed
local definition/restart, stale explicit Start refusal without fallback, late standby
Save/CAS assigned-stopped behavior. Atomic host.lock excludes local mutation and real
competing CLI Start; 0600/0700 and identity/authority injection refusal checked.
Self-review retains old unreferenced configurations on their original slot default;
explicit references never fallback. Conversation authority is equality-fenced across
bindings, conservatively not independently editable. Changed add-binding comparison
from JSON order to semantic equality; no runtime/controller added.

Evidence: `/Users/loganj/.buzz/WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/BINDINGS_FD5123/`.
Two test-extension failures are retained: stale Move fixture omitted source profile;
extra standby probe wrongly expected non-authority after restart replayed grant.
Corrected test inputs/sequencing, no production assertions/timeouts weakened.
Final strict and FULL DEFAULT concurrent installed-enabled package are next; no
suite acceptance yet. Integrated wizard/full CRUD/Goose/full parity/live provider,
normal OAuth and current production kind-40002 acceptance remain outside this slice.

## Recovery fd5123 — sole writer

Recovered local HEAD and exact remote branch at `1e6ead550e0bbf6cf8f817d07732a9b88fc84160`;
no intervening publication. Preserved seven modified files and two untracked A/B
test files from retired worker; no reset/clean or private journals. Read applicable
repository/package instructions and actual diff. Surviving delta wires selection
references into local resolution and adds lock-protected immutable provisioning;
it is not yet validated. Next: focused A/B execution, self-review, then strict and
FULL DEFAULT concurrent installed-enabled package on final candidate. Earlier
K1/F1/F2 reviews stay closed; historical startup timeout remains unclassified.

## Reusable binding execution — active implementation boundary

This continuation resolves an explicit public `harnessSetup {id,fingerprint}`
through the existing slot preparation owner; the selected binding now supplies
actual runner/ACP inputs, Save validation, cancellation and destination Move
preflight. Local immutable binding provisioning acquires atomic `mkdir host.lock`;
no key/authority copy, journal rewrite or remote executable/env provisioning.
Legacy unreferenced selections retain their slot's original default binding.
Remote `binding <id>` edits the current named configuration; Start/Restart remain
explicit. A/B external-runner + actual TUI regression is being executed before
full validation. CRUD/edit/removal wizard, integrated setup UX, Goose/live provider
and full parity remain deferred. No diagnostic iteration.

Correction: **K1 independently CLOSED at 6a5a4592**, per
`KEY_IMPORT_VERIFICATION_6A5A4592.md`: original missing/mismatch guards, hidden
exact-key import/public-only authority and real competing-start lock barriers
passed strict + 6/6 + 3/3. F1/F2 closure remains independently recorded in
`NAMED_MOVE_VERIFICATION_2151EA3A.md`. Historical review-pending text below does
not reopen these findings.

## Startup diagnostic continuation d8fe — tested partial handoff

Starting clean published 6a5a459246cbeba4647fde6572569f93ea952291. The slots
subprocess driver now keeps bounded 8 KiB stdout/stderr tails and reports actual
exit/signal/spawn error when readiness fails, instead of discarding stderr and
emitting only a polling timeout. Same 400 × 20 ms readiness bound; no timeout
inflation, serial default or assertion weakening. Early-exit and spawn-error
regressions bind this driver. Strict + driver/slots focused 6/6 pass (19.95s),
including the formerly failing standby launch. Historical cause remains
**unclassified**; this non-recurrence is not a diagnosis. Final executable candidate:
strict passes; full DEFAULT concurrent installed-enabled package **53/53 pass,
zero skips**, natural 32.062s on Node 24.15.0. The former standby workflow passes
in 19.53s within that full shape. No production startup cause was established or
patched. Evidence (focused/full logs, exact source and separate installed binary
hashes) lives in
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/BINDINGS_D8FE/`.

Concrete binding decision from source inspection: keep identity/authority solely
in installation agents + retained journals, reuse installation `setups` as local
named definitions, and add an explicit setup reference/fingerprint to nonsecret
selected configuration at the existing selection/preflight owner (not a new
registry/controller). Local mutations must acquire `mkdir host.lock` across
read/validate/write, the same exclusion owner as host startup; absence checks
alone are insufficient. Referenced edits must not relabel actual/history; stale
selection/preparation must refuse without fallback. Current host captures ONE
setup and mode in its slot closure, and Save/cancellation/Move all consult it:
merely adding CRUD or inventory options would falsely advertise A/B support.
This continuation has not yet changed that boundary or implemented the integrated
wizard/Goose path; no new binding/provider parity claim.

## Secure local key reuse increment — intentionally partial second phase

Published K1 separately as 08be5d6eb4c1a367363e16b36bb4e2824c138b0e.
This continuation adds an executable `import-agent-key` local wizard, hidden
terminal input (protected stdin supported), exact public-key match, installation
lock and shared retained-authority validation before a secret-only atomic manifest
write. It cannot mint authority, recreate unknown slots or replace existing keys.
Retained standby/source-consumed assignment is not changed by importing its key.
Actual CLI/PTY import test + real host/relay standby denial passes (1.41s); strict
passes. Initial PTY attempts failed because macOS script refuses Node socketpair
stdin; retained import-focused logs 1–4 capture this adapter failure, then fixed
adapter uses a real shell pipe. No product assertions or timeouts were weakened.
Evidence: `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/K1_928F/`.

Grounding read: HARNESS_PROVIDER_UX_GROUNDING.md. No provider abstraction or
executable/auth claim added. **Second-phase partial:** multiple reusable local
bindings CRUD, remote exact A/B selection/fingerprints, integrated initial
new/import/reuse wizard, and second Goose executable/provider/service-env path
remain unimplemented. Next executable step is local binding management + advertised
selection at the governing host preflight/configuration boundary, followed by
source-pinned Goose ACP fixture acceptance; not a design-only task or parity claim.
Final strict/default concurrent installed-enabled results will be recorded below.

## K1 retained-authority removal — explicit closure

Inspected immutable 2151ea3a: K1 was NOT fixed by the named Move commit.
`loadSlotState` now owns the existing host hydration validation (binding,
owner/agent genesis, full assignment/grant chain, selected/configuration codec).
Both host hydration and destructive local key removal use that same read-only
loader. No new assignment synthesis, no execution-holder-equals-this-host gate.
All validation precedes manifest mutation; retained journal is never written.

Actual CLI regressions reproduce missing assignment and mismatched agent genesis,
plus wrong owner/holder, malformed chain, wrong root/predecessor/grant agent and
operation. All refuse exit 1 without success text and preserve manifest, selected
journal and sibling bytes. Valid stopped, standby and consumed-source controls
remove successfully and pass the same public reopen loader. Existing real host,
relay, removal/reopen/lifecycle and both Move directions pass: focused 5/5,
natural 12.98s. Strict passes. Initial extraction strict failure (genesis local
scope) is retained in `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/K1_928F/strict.log`;
corrected strict2/focused logs alongside. No local deletion is global revocation.
K1 is checkpointed separately before the reusable-binding/import continuation.
Final full concurrent installed-enabled package validation remains due on the
final candidate; prior timeout/failure history below is not reclassified.

## Named Move F1/F2 — implemented; final validation/publication below

Starting verified clean published `7ad542080d72f5e04692a207286d20a87646269f`.
Consumed final independent NAMED_SETUP_REVIEW_4D26C2E1.md and original recovery
TUI/wire probes (not the superseded candidate-comparison hypothesis).

**F1 owner:** strict `selection` codec equality, including nested configuration and
profile fields; adjacent target-local comparison validates before separating behavior.
Local setup/prepared-input hashes canonicalize object members but preserve arrays.
Raw hash-chain/predecessor/operation/retry identity hashes are deliberately unchanged;
no saved events are reserialized/re-signed. Historical grant meaning remains readable.

**F2 governing model:** profiles stay independent immutable behavior revisions;
named launch configurations are per-agent effective snapshots, not display labels.
`named-v1` grant materialization derives source-selected public behavior + target-local
model/workspace/name as a NEW named revision: max(target CAS revision + 1, previous
named revision + 1). Legacy implicit default@1 is materialized by new Moves too;
there is no separate unnamed inventory/selected behavior path. Even unchanged behavior
receives a new revision for this Move.
Preparation validates/bounds the resulting inventory before source effects and atomically
reserves that revision with its immutable token/reply and original target candidate.
This captures the prior destination definition and prevents a concurrent later Save
from assigning different content to the reserved reference. Cancelled preparations can
leave revision gaps; reservations are not Start/assignment authority. A retry cannot
renew the token, candidate or reservation, including after restart.

Source validates/derives that same effective snapshot before Stop; its consumed grant
and outbox remain one durable write. Target compares against the reserved revision,
then commits assignment + named inventory + selected-next together, before ordinary
Start. Later standby Save retains its later candidate, advances the fence and leaves
accepted authority assigned/stopped. Legacy grants that would redefine an old
reference instead preserve the candidate and stop; historical accepted grants and
actual/history snapshots are never rewritten. Both sides need this development
materialization contract for new named Moves; no mixed-version rollout claim.

**Regressions:** original reviewer actual TUI profiled source → Destination@1/default
now succeeds WITHOUT an object-order workaround; actual, selected-next, inventory and
versioned grant all use Destination@2/Source. Original Destination@1/default remains
in preparation evidence. Reselect + Restart, both service reopens, duplicate Move/grant,
former source/target actual/history and recursively reordered real wire Move are checked.
Fresh profiled/named dropped-grant cases exercise target restart with changed inputs,
stale reservation CAS, later same-name Save allocating @3 rather than overwriting @2,
assigned/stopped acceptance, original preparation preservation and source non-resurrection.
Unit negatives reject extra/missing/different grant/selection fields and unknown versions;
arrays remain order-sensitive. Existing installed conversation fixture remains deterministic.

**Iteration evidence (not hidden):** first focused Move run 1/6: validating the local
behavior-stripped comparison still passed an explicit undefined behavior property to
`fields`; fixed at shared validated comparator by omitting that property after codec
validation. Next original Move run 6/6. First new TUI probe passed at runtime but strict
reported an unknown receipt result in the adapted test; corrected String conversion.
Focused combinations then 4/4, 3/3 and named recovery 2/2. First full DEFAULT concurrent
installed-enabled run 47/49 (34.38s): reverse Move's old revision-4 wait was obsolete
because its named preparation now reserves 4 and Start commits 5; exact wait/Stop
assertions updated to 5, not removed. Separate slots standby-launch evidence timeout
remains unclassified; no attribution to pre-existing/contention/removal without causal
logs. Focused original Move + slots 10/10 passed subsequently, not proof of that cause.
Prior removal 43/45 failures likewise remain scoped observations despite green reruns.

An intermediate final-tree strict + full DEFAULT concurrent installed-enabled run passed
49/49, zero skips/cancellations, natural 32.08s. Subsequent self-review extended the same
materialization rule to legacy implicit default@1 (otherwise that projected inventory
could still split from selected-next). Original Move tests now assert default@2 / Start
revision 3 explicitly, later-Save @3, and cancelled-reservation CAS 2. Focused original
Move + strict semantic codec 7/7 pass; existing M1/replay/Stop/Save assertions retained.
Four full attempts after implicit-default self-review were interrupted at the unchanged
110s tool bound without a package summary (`full-installed-exact`, `full-installed-publication`,
`full-diagnostic`, `full-direct-diagnostic`). Two were diagnostic-only runs; temporary
instrumentation is removed. Each reported passing installed Move/new/Move/M1/key cases
before output stopped ahead of slots. These are NOT completed/green suites. A direct
fixture-owned diagnostic finally captured slots TUI pending at step 27 waiting for
Restart revision 4 after its Restart received **revision-conflict**. The driver had
gated on private host journal revision 3, not the TUI's inventory observation; its
unhandled rejected gate also left the child UI alive until the outer tool deadline.
The bounded driver now uses actual `show` responses to observe the exact committed
revision before Start/Restart/Stop, retains all original assertions/8s observation bound,
and propagates gate errors after closing its own UI. No product CAS bypass, longer
sleep/deadline, serial masking or force-exit. This explains the captured diagnostic
journey, NOT every earlier timeout from logs lacking that evidence. Isolated slots
4/4 had also passed and is not causal proof.
Strict + DEFAULT concurrent installed-enabled full after this driver correction passed
49/49, zero skips/cancellations, natural 33.28s (`full-ui-observation.log`). Final exact-tree
strict/full validation and source publication are recorded in terminal evidence/logs.
Logs: `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/NAMED_MOVE_6AFE/`. Independent prior named
review consumed; this delta has self-review and executable evidence, NOT independent
approval, production admission, live-provider or full-product acceptance. No secrets
in output/relay; public-only key removal, local harness credentials and sibling isolation
are unchanged. **ONE next executable action:** multiple local harness bindings and
import/reuse wizard/Desktop parity, after independent confirmation of these blocker fixes.

## RECOVERED removal-public-slots — recovered clean; implementation begins

Recovered sole-writer worktree `/Users/loganj/.buzz/REPOS/beehive-cbfd9440` branch
`beehive/cbfd9440` at verified published pre-run `4d26c2e1921f3e20f57888f9124ae0611334464f`:
clean tree, no stash/untracked source, remote FETCH_HEAD identical, reflog shows only the
prior task's amend. Prior writer 4aa4e8ab cancelled with NO removal commit, push or
surviving edit; nothing to preserve or repeat. Immutable named-config review 818c3724 is
read-only and untouched. This section is written BEFORE implementation or long validation.

Planned bounded scope (no wider wizard/harness CRUD): local deliberate agent-key removal as
a retained public-only slot under the stopped installation lock (refuse when lock present —
no PID/stale-lock removal), validating public key/journal identity/stopped phase, preserving
0600/0700 and public identity/genesis/assignment/config/profile/receipt/run history plus
sibling keys. Host reopen advertises the missing-key public slot; Start/Restart reject before
spawn; Stop/Save/inspect/reconnect stay credential-neutral; remote operations never recreate
the secret. Move TO a missing-key target rejects during destination preflight before source
Stop; Move FROM a public-only source follows the existing grant contract (assignment consumed,
no key transfer/reconstruction/takeover). No remote key CRUD, controller or privileged HTTP;
no shadow identity — the execution credential becomes an optional loader. Planned validation:
actual local CLI + host/client/relay tests, focused first, then strict + DEFAULT concurrent
installed-enabled full suite on the exact final candidate (baseline 41/41).

## Removal public-slots — implemented, validated, published head in terminal result

Started published `4d26c2e1921f3e20f57888f9124ae0611334464f` (prior writer 14c23528 handoff:
uncommitted patch, strict pass, NO executable tests/commit/push). This candidate is the
commit containing this section. The prior run's drafted `remove-key.test.ts` was
**never saved to disk**; it was written fresh here, then executed.

**Implemented (validated, not re-implemented):** `removeSlotKey` refuses any
installation lock (no PID/stale-lock removal), validates public identity + stopped
slot/no actual, atomically nulls only the secret in the 0600 manifest, retains
sibling/owner/history; `addSlot` refuses recreating a public-only slot; host execution
credential is optional with missing-key Start/Restart rejection before effects, public
inventory `localKey`/readiness, neutral Stop/Save; conversation prepared credential;
CLI `remove-agent-key <dir> [public-key]` with confirmation; assignment-export works
public-only; conversation-setup/migration missing key reject.

**Real defect found and fixed at its owner:** the inherited patch's `migrateSlots`
destructure had dropped `agentSecret` from the omission list, so a migrated v2
manifest kept the legacy key inside `setups.default` — `installationSlots` silently
re-armed a `secret: null` public-only slot from that retained key, defeating removal
(Start would have launched and Move inventory never showed the removal). Fixed by
restoring the omission, plus a fail-closed `readInstallation` check that shared
harness entries never carry key material. Regression is executable, not a test workaround.

**Validation:** strict TypeScript passes; fresh `test/remove-key.test.ts` focused
**4/4** (removal journey incl. CLI confirmation/no-secret-stdout, 0600/0700,
byte-identical journal/sibling, reopen advertising, pre-spawn Start/Restart denial with
fixture-runner line-count proof, neutral Stop/Save, reconnect replay; active-lock refusal
without lock deletion, non-stopped/unknown/malformed/already-removed fail-closed, legacy
key-less remove/migrate/conversation-setup/host-startup refusal, public-only
conversation-setup refusal; Move-to-missing-key destination preflight failure preserving
the running source; public-only source Move consuming retained management authority with
destination launching on its own local key, source manifest byte-identical, consumed source
Start denied). Adjacent `slots/move/assignment/configurations` **14/14**. First strict +
DEFAULT concurrent installed-enabled `BEEHIVE_REAL_BUZZ_ACP=/Applications/Buzz.app/Contents/MacOS/buzz-acp npm test`
full run **45/45, natural exit 30.27s**. On the final tree (identical test-relevant
source; only docs edited) one full run recorded **43/45**: two PRE-EXISTING tests
(`cancellation.test.ts` same-batch Stop, `slots.test.ts` real TUI lifecycle) failed with
`Missing … evidence` timeouts — the historically documented intermittent concurrent-fixture
contention, NOT a removal regression (all four removal tests passed in that run). Targeted
rerun of both files **5/5**, then a fresh complete DEFAULT concurrent installed-enabled run
**45/45 pass, zero skips/failures, natural exit 30.19s** (baseline 41 + 4 new) under
Hermit Node 24.15.0 / pnpm 11.4.0, standalone frozen lock unchanged. Installed runtime
`10612d00…ea` and CLI `147cc2cc…5c` freshly match the retained pins. No serial mode,
timeout weakening or containment assertion change; the intermittent observation is
recorded, not silently classified as fixed. Evidence:
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/REMOVAL_PUBLICATION/` (focused-remove-key.log,
focused-adjacent.log, full-installed.log, full-installed-final.log (43/45),
intermittent-rerun-targeted.log, full-installed-final2.log (45/45), installed-binaries.sha256).

**Consumed independent named-setup review**
`NAMED_SETUP_REVIEW_4D26C2E1.md` — TWO findings remain **OPEN and deliberately NOT
fixed here** (out of this bounded task): (F1) property-order-sensitive grant equality
at `handoff.ts:32` reached from `host.ts:196-197` rejects an actual TUI profiled Move to
a named default-profile destination (do not fix only `host.ts:208`); (F2) accepted Move
updates selected-next but not the named inventory (`host.ts:266-274`), so the same
`Destination@1` can mean different instructions. This removal delta's own Move tests
use plain default selections and do not intersect those seams; no workaround was
combined into that review's approval. **ONE next executable step:** fix F1/F2 at the
natural grant/candidate owner with the review's regression combinations.

**Remaining:** local key REPAIR/re-provision reconciliation UX (removal is one-way by
design), nsec import/saved-owner verification/remote revocation UX, full import/reuse
wizard, local setup CRUD/multiple setup bindings, other harnesses/providers/presets,
mesh/compute, production admission/OAuth/live-model gates, final UX review. Removal is
a local copy deletion — not global cryptographic revocation; no nsec transfer or remote
key CRUD exists. Historical unrecorded broker failure remains unclassified.

## Named configurations 6bf9 — usable increment, local parity still partial

Started published `e70f90f9aec26248022aa1081339548e1ade9ea4`. Candidate is this
commit (`git rev-parse HEAD`); exact published head is in terminal result.

**Implemented:** existing per-agent journal/Save/CAS now owns bounded named launch
candidates (create, Save, list, select, rename, inactive removal). Names and revision
numbers are captured in immutable actual selections and survive removal/rename in
run history. Revision numbers use the host/agent journal generation, never reset on
name reuse. Existing default selection is projected without lossy history migration;
old actuals remain legacy/unversioned. One atomic persist contains registry,
selected-next and receipt. No second profile/provider/credential authority. One
harness setup per slot is exposed with nonsecret ID/kind/allowed model/workspace;
actual-run includes the shared setup definition fingerprint (excluding agent/owner
keys), executable digest and complete prepared-input fingerprint. Setup fingerprints
do not claim credential-cache generation or pin executable replacement across spawn.

**Local wizard:** `setup` directly creates shared slots; no migrate-slots paperwork
for a new host. Choose fixture or existing Buzz Agent Databricks v2, local executable,
allowed workspaces, see harness-specific sign-in/service uid/HOME/config context,
then deliberately create independent keys using that same harness setup. Optional
owner-only key/root file import still attaches standby to existing genesis. Subsequent
named configuration changes never recreate/import/copy keys. Local structural changes
require the host stopped/exclusion lock; no management HTTP, implicit lifecycle or login.
This is a smaller first-setup flow, not complete key/setup CRUD or Desktop parity.

**Standby and Move:** public Save is now allowed on an exact standby agent/host;
Start/Restart/Stop/Move remain assignment-fenced. Preparation validates the exact
observed destination candidate, with source-selected behavior authoritative. Self-review
caught the newly reachable target-Save-after-prepare race: accepting a later Save must
not strand an already consumed grant. Grant acceptance preserves that later candidate,
advances authority/revision, stays stopped and requires explicit Start. No mixed-input
launch, discarded accepted Save or source resurrection. Prepared/grant records stay
immutable and duplicate replay stays inert. Normal successful Move applies exact target
name/revision/workspace and source behavior, not a floating current candidate.

**Actual acceptance:** `configurations.test.ts` drives real local wizard and real TUI
subprocesses through two identities sharing one setup; checks 0600 files/0700 directory
and no fixture secrets in wizard stdout. X starts, creates Alternative, saves another
allowed workspace, selects default/Alternative while actual stays identical, then
Restart applies Alternative@3/exact workspace/same setup fingerprint and identity.
Y's complete journal stays byte-identical. Target Destination@2 is prepared remotely,
Start denied; stale candidate Move preserves source, exact candidate Move succeeds.
Remote rename/removal leave actual/history unchanged; unsupported selection rejects at
owner. Manifest remains byte-identical throughout remote work: no key/setup rewrite.
New dropped-grant/remote-target-Save regression proves assigned/stopped acceptance,
later selection preserved, immutable preparation and no source resurrection on reopen.
Existing actual profile/Restart/Move instruction tests retain behavior evidence.

**Validation:** final executable strict + installed-enabled DEFAULT concurrent package
**41/41**, zero skips/failures, natural exit **29.285s**. Previous complete candidate
41/41 30.746s; final only improves concise named list/current-next labels. Earlier
focused 10/11 exposed reverse-Move test constructing a stale selection without the new
configuration tag; fixture now supplies exact observed selected-next, matching the
new target-candidate contract (no weakened production checks). Prior full 38/38 and
new focused 8/8 also passed. No serial/force-exit or timeout/containment assertion
changes. Logs/source hashes/binary hashes in workspace
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/NAMED_CONFIGURATIONS_6BF9/`.

**Independent review consumed:** `PROFILE_RESTART_REVIEW_E70F90F9.md`, no supported
blocking finding, independent strict/default installed-enabled 38/38 at pinned
predecessor. Additional independent installed Restart probe establishes same signer,
new actual run/session and B instructions on replacement/next turn; actual TUI branch/
default and relay trust-boundary probes passed. Reviewer corrected an overstrict
later-turn-prompt repetition oracle, with initial failure retained. This new named
configuration/standby-Save delta is self-reviewed, NOT independently approved by that
predecessor report. No repository-wide CI or live provider approval claimed.

**Remaining:** key removal/revocation with retained public assignment/history and
public-only missing-key slot inventory; complete import/reuse wizard; local harness
setup CRUD/multiple remotely selectable setup bindings; a materially different
Goose/Claude Code/Codex path and vendor/adaptor auth, remaining Buzz Agent providers,
ten presets/custom ACP/mesh/compute; final independent simple UX review and production
kind-40002/admission/normal service-user Databricks OAuth/live exact model. Current
wizard has fixture + existing Buzz Agent path only: fixture is not Desktop harness
parity. Historical unrecorded broker failure remains unclassified. Trusted exclusive
non-cloned/non-rollback POSIX installations/shared-owner physical-host limits remain.

**One next executable step:** implement local deliberate key removal as a retained
public-only slot (not deleting the slot/journal), with stopped installation exclusion;
exercise remove → service reopen → advertised missing-key state → rejected Start/
Restart/Move without reconstruction, while Stop/history and a sibling stay truthful.

## Behavior profiles 0aa71 — implemented application and durable actual evidence

Started clean published `e6717541d4580541cd24816e062d893a49260bbc`. Candidate is
this commit (`git rev-parse HEAD`); publication head is in the terminal result.
Named nonsecret behavior profiles and explicit per-agent application are implemented,
not another Restart-only prerequisite. No architecture/host/credential replacement.

**Definition authority:** immutable owner-authenticated encrypted relay publications;
`src/profiles.ts` alone owns canonical name/parent/instructions content addresses and
lineage projection. UI and hosts replay that code, not writable prompt replicas.
Concurrent children remain explicit branches; incomplete lineage is visible and not
selectable. No timestamp/latest selection. Draft form → signed durable publication
→ separate assigned-host Save/CAS acceptance → explicit Start/Restart. Current TUI
commands: `profile-new`, `profiles`, `profile-edit <number>`, `apply <number|default>`.
Numbered profile choices are captured from the displayed list; publishing shows
observed affected host/agent associations and never changes them. No JSON/journal-ID
editing, profile provider/model/credential fields or local profile admin commands.

**Captured application:** selected-next contains the validated full immutable revision.
The existing Start/Restart/Move preparation hash covers effective behavior. Launch
never reads a mutable catalog head across awaits. Actual run retains full selection,
profile revision, explicit instruction SHA256, prepared-input hash, executable/run
and existing session evidence. Successful snapshots persist in per-agent `runs`,
unmodified by later Save/Restart/Stop; inventory exposes recent hash/revision summaries.
Old journals are not migrated or synthesized into historical proof. Default remains
absence of an override (`upstream-default`, no invented text hash). `apply default`
is explicit selected-next clearing. No destructive profile deletion is supported;
immutable revisions/history remain resolvable. Bounds: 2 KiB instructions, 1,000
versions/runs, 100 parent edges; full run history blocks Start/Restart, never Stop.
Bounded wire/Move-lineage checks prevent oversized new grants consuming authority.

**Actual external evidence:** captured instructions use established upstream
BUZZ_AGENT_SYSTEM_PROMPT and installed BUZZ_ACP_SYSTEM_PROMPT, not a new broker prompt
owner. Real two-agent TUI creates A, selects/starts X, starts Y default, publishes B,
applies B while X runs (X actual A and complete Y journal unchanged), then Restart X
with same identity/new run/process and external received B bytes. `profiles.test.ts`
proves external ACP preflight and actual launch both receive B while publication C /
Save races Restart; C Save conflicts, no mixed inputs. Stop cancels a later delayed
Restart with no revival. Canonical/tampered hash, unexpected fields, missing lineage,
branches, invalid references and stale selection tests pass. Old actual snapshot
survives Restart/Stop. Real durable-client Restart receipt loss BEFORE relay storage
recovers outbox on reopen, unchanged journal and exactly two launches (no extra one).

Actual two-host TUI Move now preserves source-selected instructions despite target's
default config; target external process records the same bytes, Y journal unchanged.
Installed conversation acceptance passes the profile marker into both actual harness
environment and the installed runtime-generated ACP prompt; three native signed
replies still obey owner/member/thread/model fences. This is installed Start/runtime
propagation, not installed-conversation Restart or live-model behavior proof. Actual
Restart uses deterministic external lifecycle/ACP fixtures with new process/run;
fixture session labels are process-scoped, not global session-uniqueness evidence.
Instruction hash covers the profile override, not upstream orientation/memory/history.

**Validation:** final strict + installed-enabled DEFAULT concurrent `npm test` **38/38**,
zero skips/failures, natural exit **29.50s**. No serial mode/test-script changes.
First full 38/38 29.78s; after history additions full 37/38 29.53s, exact failure
`broker.test.ts:33`, existing-but-empty reverse-rpc marker (`'' !== 'ok'`). Fixture
open/truncate/write vs existence-gated reader was corrected by atomic marker rename;
broker assertion/timeout and production code unchanged. Targeted broker 1/1, then
final full above. This does NOT classify the older unrecorded broker failure. New
Move boundary assertion initially read before the external fixture recorded startup;
now awaits explicit received-byte evidence before asserting, targeted 5/5. No owned
exit/ESRCH assertion changed, no broad stress campaign. All retained logs/hashes:
`/Users/loganj/.buzz/WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/PROFILES_0AA71/`.
Installed runtime and CLI hashes freshly match prior pins; binary attribution remains
separate from source. Standalone dependencies/lock unchanged. No owner private stores,
provider/login/production operations, Rust/native changes, remote secrets or keys.

**Review / next executable action:** independently review this published profile delta
TOGETHER with predecessor Restart, especially publication/association separation,
immutable captured inputs, history, Move behavior preservation and lost-receipt tests.
Self-review performed; no independent new review or repo-wide/GitHub CI claimed.
Prior CONVERSATION_MOVE review remains consumed, not reopened. Scoped live-anchor
POSIX ownership and trusted non-cloned/non-rollback installation assumptions remain;
no arbitrary escaped-tree/physical-host/partition/clone/rollback-safe claim.

Remaining scope stays visible: local key/setup CRUD + small harness-specific wizard,
named launch configurations and retention/history UI, Desktop harness/provider/preset/
custom/mesh/compute parity, final real-TUI UX/live acceptance, production kind-40002/
admission and normal service-user Databricks OAuth/live exact-model gate. Those are
not completed or replaced by deterministic profile/Restart acceptance.

## Restart continuation dad771 — partial product handoff

Started clean published `a235638e0a879c3b93bec39b8b27f415c03ab181`; candidate is
the commit containing this section. **Explicit per-agent Restart is implemented;
named behavior profiles/applied instruction revisions are NOT implemented.** This
is a working lifecycle increment, not completion of the delegated profile journey.

Restart reuses the existing encrypted relay intent journal, per-slot assignment/CAS,
operation reservation/receipt/outbox and Start/Stop ownership path. Captures selected
inputs and fingerprints; checks local setup/key/executable/workspace before Stop.
ACP prerequisite probe is identity-free and separately retained while the old run
stays owned. Fully stopped probe, unchanged phase/fingerprint, complete old owned
exit, cancellation recheck and final fingerprint precede fresh actual-run launch.
Probe failure preserves the existing actual run; teardown uncertainty quarantines
and retains handles. Valid Stop cancels queued/in-flight Restart; invalid authority,
revision/body/previously-reserved ID cannot retract it. Save is serialized (after a
successful Restart its old revision conflicts), never mixed into captured inputs.
Assignment/root/grants remain untouched; no setup/key regeneration or schema rewrite.
No promise of future provider/relay readiness from the independent prerequisite.

Executable acceptance: `restart.test.ts` real relay/host/external fixtures verifies
same identity/assignment, new actual run ID, duplicate replay with byte-identical
journal, deleted setup preserving running actual without recreating key, same-batch
Stop, ACP rejection preserving old run, invalid Stop inertness and authorized Stop
interrupting four-second preflight with no revival after its original completion time.
`slots.test.ts` actual TUI now runs Restart X while X/Y both run; X gets a new run,
Y's complete receipt/revision/run journal remains byte-identical. These are fixture
proofs, not actual applied behavior instructions or installed conversation Restart.

Final executable strict passes; installed-enabled **default `npm test` 35/35**, no
skips/failures, natural exit, **29.32s**. Earlier increment default full 34/34 30.55s;
then targeted Restart/slots 6/6 17.99s. No serial invocation in this continuation,
no changed test script/global timeout/ESRCH assertion, and no failed test attempts.
Self-review added retention of failed probe teardown and a phase fence so heartbeat
quarantine during preflight cannot be overwritten by a new launch. Binary hashes
separately rechecked and match the prior pins. Evidence:
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/RESTART_DAD771/` (workspace absolute prefix
`/Users/loganj/.buzz/`) holds default-full, targeted, check-final, default-final logs
and installed hashes. No fresh dependency install was needed; lock unchanged.

Consumed fresh independent `CONVERSATION_MOVE_REVIEW_A235638E.md`: no new blocking
Move defect, M1 resolved through original-derived reproduction plus postacceptance
restart sibling; independent default concurrent 33/33 (31.956s), additive installed
Move 2/2 (26.987s), grant-time positive PID/ESRCH and exactly one pregrant source AUTH.
This review covers a235638 Move, **not the new Restart delta**. Prior concurrent
broker failure at 1990.69ms remains historically unclassified: original log has no
stack/final summary. Neither reviewer nor current default green runs classify/fix it.
Previous serial 33/33 150.15s remains valid only for its stated older serial mode.

**ONE next executable action:** implement the remote nonsecret named behavior-profile
owner and revisioned selected-next/applied-instruction snapshot, then extend the
existing two-agent TUI Restart journey to prove actual instructions (not just labels).
Do not treat this narrower increment as scope removal: named launch configurations,
profile CRUD/use semantics, Restart lost-receipt/save-race/installed-conversation
specific acceptance, local key/harness CRUD/guided setup, remaining Desktop harness/
provider/preset/custom/mesh/compute, final UX, production kind-40002/admission and
normal service-user Databricks OAuth/live exact-model gate remain. Source-grounded
matrix is retained in README and workspace HARNESS_PROVIDER_UX_GROUNDING.md.

## Identity Move ff994 — executable completion checkpoint

Candidate is the commit containing this section; publication/remote head is recorded
in the terminal result. Started clean published `97a9050adbabd09b868bdd55ce3af333c8ed0e9f`.
The previous fixture-only/circular exact-ready gate is removed, not relabelled ready.

**Two gates at the host/broker owners:** preflight checks locally provisioned same key,
setup, selection, canonical executable/hash/script/workspace fingerprint; bounded
identity-free installed runtime/CLI help checks and an independently owned ACP
prerequisite greeting probe (no signer/relay/tools) complete and fully stop before
prepared publication. Catalog fallback alone is never auth or readiness proof.
Inventory labels prerequisite preparation separately from actual conversation readiness.
After durable source exit/consumption/grant the target accepts once, freshly launches
and requires the unchanged broker's exact model/session/response completion. Failure
keeps target assignment and source revocation; no transfer of keys/workspace/session.

**Observable acceptance:** `test/installed-move.test.ts` uses TWO real host subprocesses,
SAME fresh locally provisioned signer, actual installed buzz-acp and Buzz CLI, isolated
actual conversation relay, deterministic existing ACP harness. Source answers first
owner event; grant interception observes source stopped/consumed and source harness,
TERM-resistant descendant and tool shim ESRCH BEFORE forwarding grant / target actual
spawn. Target answers a second channel then a later turn in the first thread with
canonical Schnorr signatures, exact channel/parent/non-owner member recipient, native
CLI read/rejected-mention behavior and post-grant actual model/session evidence.
Missing target setup, unsupported selection and rejected independent ACP preparation
preserve the running source. Actual model rejection injected ONLY after grant leaves
target assigned/stopped/null actual, no target reply, source Start denied. Source-Y
byte-identical, cancellation, lost grant/receipt, historical/duplicate grant and
source-restart evidence is reused from the full production Move suite, not re-created
with a second signer in the installed conversation test.

**Review consumed:** `MOVE_REVIEW_97A9050A.md` M1 was blocking and is now fixed: existing
preparation/grant-acceptance token cannot be replaced across replay/restart. Expired
live readiness rejects preparation retry (new operation needed); exact original
consumed grant still assigns target and leaves it stopped. Added changed-target-inputs
+ graceful target restart + retained prepare-before-grant relay-history regression;
original preparation deep-equal, duplicates inert and source remains denied across
restart. Targeted Move/fixture **6/6** passed after this correction. This is author
regression evidence, not a claimed independent re-review of the new delta.

**Containment judgment:** consumed reviewer source analysis and exact grant-delivery
runner/ready TERM-resistant-child ESRCH probe. Current barrier is supported for
exclusive live-anchor POSIX in-group execution, including separately broker-owned
harness/tool groups and childless shim absence checks. Unknown/missing anchors remain
fail closed; no numeric PID recovery grants kill authority. Historical old unrecorded
PID cause remains unclassified. Reviewer separately captured empty PID bytes -> 0,
not a live descendant. Fixture now awaits successful spawn, validates a positive PID
and atomically publishes it before cancellation; absence assertion and bounded failure
diagnostics unchanged. This is not arbitrary escaped-tree containment, physical-host
proof, exactly-once/partition/clone/rollback safety. No broad historical stress campaign.

**Validation:** Node24.15.0, existing standalone dependencies/lock unchanged. First
candidate strict + installed-enabled full **32/32** passed in 24.50s. After M1 and
expanded negative acceptance, default concurrent full run recorded a broker failure
and did not finish within the 90s tool deadline (no final summary; slots results absent).
Its exact failure cause is unclassified, not silently called fixed. Targeted unchanged
broker passed 1/1. Final exact executable state: strict passes; complete installed-enabled
package with explicit `--test-concurrency=1`, no force-exit, **33/33**, zero skips/failures,
150.15s. Serial validation avoids concurrent fixture contention, does not classify the
failed concurrent invocation or establish default-concurrency reliability. No product
sleep/deadline/absence assertion was weakened. Initial rejection-fixture iterations
failed because fault injection at AUTH was too late (target harness already spawned);
fault moved to pre-delivery grant interception, proving actual postgrant rejection.

Evidence: `/Users/loganj/.buzz/WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/IDENTITY_MOVE_FF994/`
contains check/check-final, first full, failed concurrent final, final serial full,
installed-targeted and M1-targeted logs; binary hashes freshly match existing runtime
`10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`
and CLI `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`.
Installed binary identity is separate from TS source identity. No actual Databricks,
provider/login/community operations, production kind-40002 admission, owner private
stores, Rust/native changes, repo-wide/GitHub CI, PR/merge/release or full parity claim.

**ONE next executable action:** independently review the published identity-conversation
Move delta and its 33-test acceptance, focusing on prerequisite-probe ownership and
fresh postgrant readiness (M1 reproduction is now a permanent regression). Then broader
profiles/key CRUD/Restart/harness/provider/preset/mesh/compute/live UX scope remains as
listed in README; normal service-user Databricks OAuth/model and production admission
are separate real-user gates, not missing conversation Move engineering.

## Identity Move continuation ff994 — recovered / design decision

Recovered clean `97a9050adbabd09b868bdd55ce3af333c8ed0e9f`. Read current
handoff/host/ACP/conversation/owned/supervisor and installed conversation fixture.
The earlier “exact-ready preflight” rationale is superseded: preflight is local
prerequisite validation, not future same-run readiness. Actual conversation readiness
still belongs to the post-grant broker's exact model/session/completion evidence.
No pre-grant identity-bearing conversation is permissible. Current implementation
still blanket-refuses conversation Move; this entry does not claim acceptance.
Production Stop uses a retained IPC group leader, self-group TERM/KILL and waits
for group ESRCH; missing live anchor or unknown exit fails closed. Historical
invalid fixture PID observations are not themselves a demonstrated barrier defect.
Independent current transaction/barrier review is pending; no broad rerun campaign.

## RECOVERED 9ff7 — executable fixture transaction, partial product handoff

Recovered actual HEAD and actual remote both at
`8ba5fa9cd474aab5ce48fa255b194f8d3173aec1`, with no prior transaction commit/push.
Preserved dirty checkpoint/cli/host/intents/protocol and untracked handoff/move test.
A RECOVERED entry was written before implementation or long validation. The commit
containing this section is the new executable candidate (publication recorded in the
terminal result). Historical sections below are superseded by this current state.

**Recovered:** source reservation, target preparation, source Stop, durable one-way
consumed authority + exact grant/outbox, validated predecessor chain, idempotent target
accept/fresh Start, TUI Move intent, three initial tests. **New:** validated Stop/Save
before cancelling Move; exact pending Move retry republishes prepare rather than a
terminal interrupted receipt; non-fixture/conversation Move explicitly refused before
source effects because metadata preparation is not exact-model ready. Shared status,
rows and retry confirmations now identify host + full agent + operation; Move includes
endpoints. Source Start never resumes after consumption, including service restart.
No source-validator relaxation without successor chain; no authority rollback.

Observable tests: actual two-host subprocess + actual TUI Move X leaves source Y's
journal byte-identical; standby denied before grant; wrong target revision preserves
running source; reverse successor rejects historical grant. Fault-injected relay
loses grant and source receipt before receipt storage; inspect recovers outbox once.
Target key deletion and prepared setup change after grant both leave target assigned
stopped and source irrevocably denied, including after source service restart. Delayed
prepared replies cover exact pending retry, valid Stop/Save cancellation, malformed
Stop/unsupported Save inertness and late Start denial. Target admission failure here
is not a claim of a live-provider launch failure or moved conversation response.

Consumed `ASSIGNMENT_SLOTS_REVIEW_8BA5FA9C.md`: F1 fixed, actual reopened two-agent
same-host/action/revision blocked-operation TUI regression checks each row and retry
confirmation. Declining either sends neither; immutable intent/receipt fences retained.
Consumed `RECOVERY_VERIFICATION_8A67B967.md` findings remain closed, not pending.

Consumed new `DESCENDANT_EXIT_EBC8B3CB.md` report during this run. It reproduces the
invalid fixture PID failure signature and reports group/member teardown probes green;
historical raw PID/errno were not captured, so invalid PID versus PID reuse remains
unproved. **Containment gate remains OPEN, not cleared by these runs.** At the exact
reply-tool absence assertion, added raw PID, errno, spawn/error provenance and bounded
per-PID state diagnostics on recurrence, retaining ESRCH requirement and no extra
sleep. No owned.ts/supervisor teardown change or reconstructed PID kill authority.
Existing live anchors remain the only teardown authority; stronger full-lifetime
provenance and the historical observation still require reconciliation. Do not call
this preview safe Move or an exactly-once/partition-safe transfer.

Validation evidence: `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/MOVE_9FF7_EVIDENCE/`.
Recovered strict + Move **3/3** passed; new targeted TUI/Move **5/5** passed, expanded
Move **4/4** passed. First candidate full installed-enabled **30/30**, zero skips,
19.90s (`check.log`, `full.log`). Late self-review then corrected pending Move retry
semantics; targeted **4/4** passed (`pending-retry.log`). Final-source strict/full
**30/30 pass, zero skips**, 19.12s (`check-final.log`, `full-final.log`). No failing executable run occurred in this
recovery; earlier worker test history is unknown. Initial discovery directory scan
timed out; no state mutation. Prior historical failing suite remains documented below.
Installed binary hashes are separate from source identity; both freshly match the
retained pins (`installed-binaries.sha256`).

**ONE next executable action:** run the instrumented installed-enabled full suite at
this immutable candidate under the original contention configuration, retaining raw
failure capture if it recurs, to reconcile the still-open source-exit observation
before promoting fixture Move or implementing exact-ready conversation/provider Move.
Do not repeat resolved slots-label or R1/U1 reviews. Broader gate also needs independent
review of this new authority transaction; no installed moved-identity reply attempted.

Scope: dedicated trusted non-cloned/non-rollback installations/exclusive supervision;
shared-owner signatures bind trusted protocol traffic, not physical host attestation.
Root/assignment persists stopped. TUI sends intents only; hosts share existing
management relay. No keys/workspaces/sessions/credentials transferred or regenerated,
no unreachable-source takeover, new controller/registry/HTTP route, provider/login,
production kind-40002/admission/OAuth proof, Rust/native or owner-store access.
README parity matrix retains profiles/key CRUD/Restart/harness/provider/preset/mesh/
compute/real UI/live acceptance. No repo-wide CI, PR, merge or release claimed.

## Move continuation 84e0 — implementation in progress

Starting exact published `8ba5fa9cd474aab5ce48fa255b194f8d3173aec1`.
Implementing per-slot durable reservation → target prepared → source exit →
outgoing-consumed (grant and outbox in one journal write) → incoming-accepted →
fresh ordinary Start. Genesis stays pinned; only prefix-extending validated grant
lineage can replace initial-host authority. Preparation is not authority. Save/Stop
invalidate an outstanding source reservation; post-grant failures never undo it.
Next code path: host slot relay exchanges and immutable local preparation digest,
then actual two-host execution tests and TUI destination selection.
The descendant-exit report is not yet present at its supplied public path. Existing
25/25 is not resolution; final safe/conversation Move acceptance remains gated on
that investigation and causal regression. No containment changes in this stage.

## Slot continuation e0a5 — working single-installation slice

Started from verified published `ebc8b3cb24fe0eefe5b8f239a88555786eadbf14`.
The candidate is the commit containing this checkpoint. One installation now owns
ONE exclusion lock, owner identity, management WS connection and heartbeat. The
existing admission/receipt/run state is an explicit in-process slot per persistent
agent, not N daemons. Routing happens BEFORE operation-ID reservation/cancellation;
queues, retractions, receipts, revisions, assignment and actual-run ownership remain
per agent. Selected-next still never mutates actual-run. No validator relaxation.

`src/slots.ts` provides an explicit stopped `migrate-slots` upgrade: atomically
replace setup.json with version-2 installation inventory (shared harness setups +
agent keys/setup references), leaving the original journal byte-identical in place.
No key generation, copied key, re-enrollment or wiped history. Legacy single-slot
hosts remain supported without silent upgrade. Local `add-agent` creates a new key
(or explicitly imports a matching public-root standby), reuses host-owned default
harness, writes its inert per-agent journal before activating the manifest entry,
and refuses partial leftovers. Maximum 32 slots. Shared auth/conversation setup is
host-owned. Multiple selectable harness inventories and key/profile CRUD remain
unfinished, not hidden remote setup APIs.

`host.close` attempts every slot via allSettled, preserving successful siblings'
stopped truth while quarantining/reporting uncertain execution and retaining the
single host lock. Shared management client reconciliation now queries every
unresolved host/agent pair, not merely the last agent per host. All slots hydrate
before dialing: initial relay history coalesced with socket open is accepted before
the ready-promise continuation, rather than discarded. Existing receipt fingerprint,
policy-blocked unchanged retry, admission cancellation and no late revival retained.

Production-bound `test/slots.test.ts` proves:
- ONE real host subprocess + ONE management socket hosts X,Y. Actual TUI subprocess
  selects exact numbered rows, Save/Start both, Stop X; ambiguous host-name shortcut
  rejected. Y's journal stays byte-identical across X Stop, including receipt,
  revision and actual run. Same operation ID on different slots is independent.
- Both delayed external ACP Starts enter transitioning concurrently with a shared
  operation ID. Stop X cancels X while Y remains pending and later completes. Replay
  returns their distinct terminal results without either journal changing.
- Wrong-agent/host operations cannot affect a run; actual host WS loss/recovery
  replays each slot truthfully; graceful service restart retains assignment and
  reports stopped instead of phantom live execution. Same-key standby elsewhere
  still cannot Start. No transfer or source-unreachable takeover.
- Explicit migration after real Start/Stop preserves revision, two operations,
  outbox and assignment byte-for-byte. Unknown first slot makes close fail/retain
  lock but does not skip teardown of a healthy running sibling.
- Actual management-client reopen/reconcile queries each unresolved slot; starting
  the real host later consumes initial relay history and resolves both intents.

Final executable candidate: strict TypeScript + installed-enabled full package
**25/25 pass, zero skips**, 17.29s. Logs in workspace
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/SLOTS_E0A5_EVIDENCE/{check.log,full.log}`.
Single-process proof logged public identities
`d2c1dbd61931eed22800b45de0cf422bd3f5e70b7585ec6f789e8124f2f21a9e` and
`ab3c21202f172cbc9d5ae1e07d5d0021ed9a34c1d56130af24124220c3df8e83`.
Installed binaries freshly rehashed, both match retained pins below. Existing
installed signed three-conversation seam also passed; slots themselves use external
lifecycle/ACP fixtures, not simultaneous live-provider conversations.

Iteration failures retained honestly: the new TUI test initially timed out (twice,
one instrumented) because the test driver assumed a prompt ended a stdout chunk;
async status coalesced after it. Fixed the driver to consume prompt positions, no
product wait/sleep or assertion weakened. New reconciliation test then exposed
initial-history loss (one failure plus diagnostic confirmation); fixed at host
hydration/connection owner. These are distinct from the earlier unresolved
`reply-tool.test.ts:68` descendant observation. No containment code/assertion changed;
this 25/25 pass does NOT resolve that observation or prove Move-safe exit.

Consumed independent `RECOVERY_VERIFICATION_8A67B967.md`: old frozen R1/U1 both
verified fixed, original replay reproducer and conflict/later-run controls green,
actual TUI byte-identical once-only retry/outbox recovery green, strict + installed
19/19. This closes the old recovery review, not new slots review or descendant exit.
The separate bounded descendant investigator has not delivered its causal report
at this checkpoint. Trusted non-cloned/non-rollback installations and exclusive
supervision remain assumptions; shared-owner signatures are not host attestation.

**Move is NOT implemented and remains gated. ONE next executable action:** consume
the bounded descendant-exit report and bind its causal finding (or remaining probe)
to the actual owned-containment seam before implementing the source-consumed Move
transaction below: target preflight, complete owned source exit, atomic one-way
revocation/exact grant+outbox, idempotent target accept/fresh launch. No initial-host
validator relaxation, timeout resurrection, key/workspace/session transfer or new
controller/HTTP lifecycle service. Request independent immutable slot UX/correctness
review alongside that continuation; do not repeat closed old R1/U1 review.

Full remaining parity stays in README: Restart, profiles/keys/metadata/history,
selectable harness/provider/preset/mesh/compute inventory, live model/admission,
retention and recovery. No owner credential/profile/OAuth reads, production/native
or Rust edits, full repo CI/GitHub CI, PR, merge or release. Publication uses existing
approved GitHub branch, configured Logan Johnson author/committer + DCO, no invented
cryptographic signer.

## Assignment foundation handoff — delegated fb23e116

Governing boundary: key possession is not assignment. Provisioning must create a
public pinned genesis (owner, agent, initial host, random enrollment ID) and an
explicit durable per-agent assignment before any host can Start. Host startup must
never recreate a missing authority journal. Existing journals require explicit
local stopped-state migration, not implicit re-enrollment. Import attaches to an
existing public genesis and cannot root authority at the importing installation.
Dedicated non-cloned/non-rollback installations and trusted one-time local enrollment
remain assumptions; shared-owner signatures are not physical-host attestation.

Planned transaction successor (NOT implemented): reserve exact source revision and
immutable target inputs; relay target preflight; confirm all source owned groups
absent; atomically consume source authority plus exact destination grant/outbox;
accept once at destination before fresh launch. No timeout/receipt loss rolls back
a grant. Unreachable source blocks transfer even when last observed stopped.

Implemented bootstrap fence in `src/assignment.ts` and `src/host.ts`: new local
provisioning pins explicit initial authority; startup cannot recreate missing
journals; explicit lock-protected stopped legacy migration preserves history.
CLI standby import binds the existing public root and identical locally provisioned
key; root export never exports keys. Host admission and early Stop cancellation
both check durable assignment. Source Stop/restart preserves assignment. Local key
absence blocks future Start without restoring hydrated keys; Stop stays available.

TUI inventory now keys by host AND agent, groups identities with `agents`, and offers
numbered exact host/agent selection plus backward-compatible unique-host selection.
`assignment.test.ts` uses THREE distinct real TS host processes on one isolated WS
relay: source/standby share X; a third installation holds Y. Two identities run under
separate assignments and actual TUI subprocess selection shows both. Standby and
cross-agent Starts fail, source restart retains assignment, unreachable source cannot
authorize standby, deleted setup stays deleted, missing journals/migration fail closed.
This is not multiple slots on one host, nor an actual transfer. No duplicate daemon
directories are presented as slots. No Move protocol/interface stub is exposed.

Validation: strict TypeScript passed. Full installed-enabled package suite repeated
on unchanged executable candidate: **21/21 passed, zero skips**, 16.72s. First full
run was 20/21: existing `reply-tool.test.ts:68` observed a descendant PID still present
after group Stop; isolated rerun passed and subsequent full run passed. No test was
weakened and no containment implementation changed. That intermittent observation
is unresolved, not claimed fixed; retain it for investigation before Move exit proof.
Both installed executable hashes freshly match retained pins. Evidence directory:
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/ASSIGNMENT_FB23_EVIDENCE/` contains `check.log`,
`full.log` (failure), `check-final.log`, and `full-repeat.log` (21/21). Existing installed
signed-conversation tests passed, but this new assignment test is lifecycle fixture
only; no destination conversation or live provider acceptance is claimed.

Move and multi-agent slots on one installation remain gated. The current validator
intentionally requires assignedHost == genesis.initialHost; do not merely relax it
to simulate Move. Replace it with verified consumed-predecessor/grant-chain authority
only when source revocation, outbox and target acceptance are implemented together.
One dedicated owner scopes this development protocol; IDs route trusted installations,
not independent host attestation or production-community enrollment. No timestamp,
TTL, election, direct HTTP lifecycle route, remote secret or automatic takeover added.

**ONE next executable action:** introduce per-agent slots under ONE installation lock
and ONE management connection, preserving this bootstrap fence and receipt/admission
state per slot; prove X and Y coexist before building the source-consumed Move
transaction specified above. Then implement preflight/exit/grant/accept/fresh launch
and failure/replay tests, including investigation of the intermittent descendant
observation. Do not treat this prerequisite candidate as completed two-host Move.
Remaining profiles/key CRUD/Restart, richer setup/provider/preset/mesh/compute parity,
real UI/live auth acceptance, retention and trusted-scope recovery remain in README.
No production services, owner stores, OAuth/provider access, Rust/native edits,
repo-wide/GitHub CI, PR, merge or release. Author/committer Logan Johnson + DCO
verified; no cryptographic signer configured.

## Published continuation 4341 — U1 policy recovery

UX source committed/pushed as `4fa4fd146961f91fdace04f3c6fb59e6300e2473`;
exact origin head and clean worktree verified after push. This documentation-only
follow-up records that immutable tested source head.

R1 was separately committed and pushed as
`64305a7a0020da0faa599b46be37b538b9533945`; origin branch verified at that exact
head before this UX checkpoint. No reset/reimplementation or additional writer.

TUI now offers numbered `operations`, `reconcile`, and informed `retry <number>`.
Reconcile explicitly reconnects, reads relay history and queries host outbox via
existing inspect messages; it does not replay blocked work or assert policy repair.
Host inspect republishes durable terminal receipts so a receipt lost before relay
storage remains recoverable. Retry sends the original retained envelope ONCE with
unchanged ID/signature/ciphertext/body/fingerprint/revision. The durable policy block
is deliberately NOT removed: socket success does not authorize blind future retry.
Only a matched terminal receipt resolves pending work. Completed/known rejection/
UNKNOWN remain distinct, historical results are labelled separately from current
host inventory, and the confirmation explains policy repair, uncertain prior effect,
permanently invalid events and no remote cancellation. Repeated 1008 remains blocked.
No replacement ACK/control service, re-signing or host-precondition change.

Acceptance is **actual TUI subprocess**, real package client/relay/host, fresh keys
and fixture runner, with test-owned loopback WS fault placement (not a model or
production policy test):
- Policy denial before Stop publication -> explicit retry under continued denial ->
  two denials total; correction/reopen sends zero blocked commands; reconcile sends
  zero blocked commands; numbered confirmed retry completes original Stop once.
  Three total attempts all byte-identical, one Start plus original Stop in host
  operations, final stopped/revision2.
- Committed Stop receipt dropped BEFORE relay storage -> 1008 block -> corrected
  reopen queries host outbox and recovers original accepted result without a second
  Stop attempt. Final revision2; completed entries reject further retry.
- Reconcile after completion does not duplicate effects. Intentional Quit in each
  subprocess exits 0 with independent-host-lifetime message, no spurious UNKNOWN.
- Existing lost-Start/crash/receipt negatives and installed cross-conversation seams
  remain green; predecessor independent seven crypto/receipt/perms negatives reused.

Final semantic candidate: strict TypeScript + full installed-enabled package suite
**19/19 pass, zero skips**, 15.64s. Evidence in workspace
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/CONTINUATION_4341_EVIDENCE/ux-check.log` and
`ux-full.log`; production-bound regression source `test/policy-tui.test.ts`.
Binary hashes freshly rechecked and match the two pins below. No dependency changes.
No repo-wide just ci, GitHub CI approval, production admission, real provider/OAuth,
PR/merge/release, or full delivery claim. Tested source is the immutable `4fa4fd146961f91fdace04f3c6fb59e6300e2473`
listed above; the following checkpoint-only commit does not change executable source.
Configured Logan Johnson author/committer + DCO verified before push; no crypto signer.

Completed independent TOOLS_RECONNECT and INTENT_TUI reviews are consumed. The new
UX delta has self-review and executable acceptance, not yet a separate immutable
independent review. This is not a reason to leave the broader build waiting.
**ONE next executable step:** implement conservative same-identity two-host assignment /
Move with multi-host/agent selection, preserving remote-only normal configuration and
lifecycle and refusing unreachable-host takeover. Retention1000/pruning, profiles/key
lifecycle/Restart, broader harness parity and live-production gates remain unfinished.

## Continuation 4341 — R1 reconciliation

Recovered d12ed7b4e08501e2e5829da80689c9759b32bd49 plus the prior worker's
uncommitted host/TESTING/admission-cancel changes. Fresh substantive review found
an additional pending-ID gap: a same-batch Stop reusing the queued Start's ID
could retract admission although its own eventual receipt was operation-id-conflict.
Receive-order reservations now fence that early authority, alongside durable IDs,
agent/host/revision/body checks. Added conflicting-ID and invalid-body batch controls.
Executing ACP cancellation may surface `ACP session unavailable`; after verified
teardown the receipt now consistently names concurrent Stop cancellation. No sleeps,
replay exception, stale-revision bypass or changed command preconditions.
FIFO Stop handling clears its retraction before future Starts; terminal historical
IDs cannot retract again. Close drops not-yet-handled messages (UNKNOWN, replayable)
and awaits executing admission/owned teardown; no new close semantics claimed.

Fresh strict TypeScript and full installed-enabled package suite **18/18** passed
on this candidate (Node24.15.0). Exact log:
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/CONTINUATION_4341_EVIDENCE/r1-full.log`.
One initial targeted run exposed the cancellation wording variation above, corrected
before this full run. Source R1 checkpoint is the commit containing this section;
publication/head will be recorded in the next UX checkpoint.

Completed independent `INTENT_TUI_REVIEW_D12ED7B4.md` is consumed: strict16/16,
36-file byte match, seven receipt negatives, real PTY lost-Start recovery and safe
Quit; no additional blocking journal correctness defect in trusted non-cloned /
non-rollback scope. U1 is a concrete policy-correction recovery UX gate, NOT pending
review: implement supported reconcile and explicit unchanged retry next. Earlier
pending-review/future-recovery notes below are historical, superseded here.

## Local management-client journal continuation 90950bdd

Started verified published HEAD `6f893f51d20d93ff25770b66f3b52b50bcff151e`.
`src/intents.ts` is LOCAL TUI/client state, not a controller process/service.
No dependency, private-host-key, ordinary conversation permission or runtime change.
TUI automatically creates owner-only `management-intents/<relay+owner digest>/`
beside the existing operator identity. Each submitted operation is encrypted/signed
ONCE, fsynced with an exclusive immutable intent file before any send, and replayed
on bounded WS reconnect or ordinary reopen. A duplicate operation ID cannot prepare
a replacement. Receipts are encrypted in separate exclusive files; stale publication
observations cannot erase a terminal result. 1,000 intents maximum; no pruning yet.

Host receipts now include the existing request fingerprint. Matching requires the
actual owner decryption/signature, exact relay journal scope, host, agent, operation
ID and fingerprint. Host outbox/operations remain lifecycle truth; no second admission
model. Relay echo is publication evidence, not host admission/completion. The UI
shows pending/completed/failed/unknown, with `operations` for retained details, and
prints host/action/result rather than requiring operation IDs. Quit cancels transport
reconnect only; it never cancels an operation or stops a host. Save remains selected-next.

`test/intents.test.ts` proves actual socket loss before publication, reopen applying
the original Start once, retained terminal results, unrelated/duplicate receipts not
closing another intent, permanent host rejection, and relay-side socket termination
when delivering a Stop receipt AFTER the host journal has committed stopped. Automatic
client reconnect recovers that receipt. A separate actual Node subprocess exits 23
without running the network event loop or graceful close after preparing an intent;
reopen publishes byte-identical envelope, verified against the relay's stored log.
Tampered signature fails closed. Relay policy close 1008 remains UNKNOWN and durably
blocks blind automatic replay across reopen; no fabricated host failure. Normal
network loss retries eight times; exhausted/offline pending work remains durable.
File writes propagate errors instead of being swallowed as malformed network input.

Evidence: strict TypeScript + full installed-enabled package suite **16/16** passed.
After human-readable connection/status display refinement, strict + targeted real
TUI/client suites **5/5** passed; final source strict + full default suite
**15 pass / 1 installed opt-in skip** passed (the unchanged installed path was not
repeated). Prior installed runtime evidence retained; this run
also produced three verified fixture replies from agent
`b727bb8ae1e277a522f8dbad00aae15cc7b71e39c57d43e7f5d532529e8087aa`.
No real provider/admission calls, production activity, full repo CI or PR/merge/release.
Completed preceding broad-tool/reconnect review is consumed in continuation 4341 above. Existing B1/D1-D3 conclusions retained, not
reopened or treated as approval of this new journal delta.

Limits: relay URL is the community scope in this development topology, not a stable
production community identifier. Publication echo observation is session-local; terminal
host results persist. There is no relay per-event rejection/ACK protocol: policy failure
is conservatively UNKNOWN/blocked, not claimed failed; guided recovery is now implemented in continuation 4341;
journal retention remains future work. Pre-upgrade host receipts lack fingerprints
and cannot complete new journal intents. Arbitrary disk rollback, malicious local
operator, simultaneous UI action coordination, log-full recovery and host SIGKILL
recovery are not proved. Single-operation exclusive files prevent conflicting-ID
replacement and receipt overwrite, not distributed UI serialization. Crash test is
process exit without cleanup, not machine power-loss proof. Stop-during-Start remains
covered by unchanged full-suite regressions; no claim that this new test automates the
entire interactive crash workflow.

**Historical next step (completed in continuation 4341):** independent journal review
and real TUI policy-recovery acceptance. Current next step is two-host assignment/Move.
Full remaining parity scope below remains active, not declared complete.

## Recovery 4200a3ab

Recovered clean HEAD `f8f161ffc10f274e13b9e1cd984ca8ffc2897cfd`, not
`023c9274767ef50fa0f5b37ef1883336f8be59fd`: the interrupted worker had already
committed the multi-conversation/tool slice and its checkpoint. No untracked
source or pending diffs survived. Remote branch lookup returned no ref and GitHub
PR lookup for this branch returned `[]`; no previous push/PR found. All six task
commits retain Logan Johnson author/committer and DCO. Recovery preserves that
implementation rather than repeating it. Fresh recovery validation passed strict
TypeScript and the installed-enabled full suite **12/12** on that source HEAD.
Rehashed binaries match the pins below. Fresh signed reply IDs were
`4629e63f8e838101943e41ab056c37610371f288ac51d63a0e3c3120c857b05f`,
`e836d1d91e6cf93ae81fe4321071b95543bfc34fc5a5ee287eec7f1a1fdb3d01`,
`90389c57ba6f33f4443cbb77cd7352c0ad1133f58da1f0741216f5d78a0a29a0`,
all from agent `ea79535a7d50890ba23bbc960ffd780acecd31f9c9a1dd975dd0d7fbbac419f8`.
No recovered PID was used for cleanup and no private run artifacts were read.

New implementation commit `998e919a9b26af0ec3b15eefba539661380eeed7`
was pushed successfully as the first publication of `beehive/cbfd9440` to
`https://github.com/block/buzz.git`. This checkpoint-only follow-up retains that
source. No GitHub or NIP-34 PR has been created; repo-wide `just ci` remains outstanding; subsequent independent
reviews are consumed in continuation 4341. `buzz pr open --help` requires a NIP-34 repository
owner pubkey/id; neither was inferred from GitHub ownership. Any later Buzz PR
must carry channel `f45d3304-dcf0-44e8-a46d-bcd63b235fbc`.

## New continuation: host management transport recovery

`client.ts` now supports opt-in bounded reconnect (eight attempts per lifetime,
100ms exponential backoff capped at 2s, 2s handshake deadline). Initial connection
failure still rejects readiness and unwinds the host lock; intentional close
cancels pending retries. Only the host opts in: TUI reconnection and durable
controller intent/ACK tracking are NOT implemented by this change. Exhaustion
leaves management disconnected; host lifetime/ownership remain independent.

The host replays durable receipts and publishes fresh inventory on reconnect.
Replayed relay command history still passes through the existing operation-ID,
fingerprint and revision guards. `reconnect.test.ts` drops the actual host socket,
publishes Start while disconnected, observes acceptance after automatic replay,
drops the socket again after commit, and verifies the identical receipt, actual
run and single revision/operation survive. Stop still tears down the owned run;
intentional host/client close cannot reconnect. This proves relay-persisted
intent recovery, not recovery of controller intent that never reached the relay.
The existing receipt outbox is retained, not ACK-pruned. No protocol change or
new remote/local lifecycle endpoint. This delta subsequently received TOOLS_RECONNECT review; R1 is resolved above.

## Implemented: ordinary multi-conversation Buzz CLI tool authority

Host -> installed buzz-acp -> childless ACP shim -> separately host-owned ACP
harness -> childless MCP shim -> host-resident TypeScript MCP adapter -> separately
host-owned installed Buzz CLI. No second chat/agent runtime; no Rust/client edits.

`conversation-setup` optionally provisions only a canonical installed Buzz CLI
executable. `buzz({argv, stdin})` exposes its native command contract to the model,
including native help. No local channel/parent/recipient grants and no prompt
scraping in the host. One persistent agent can answer different conversations
without another host-admin step. Old single-thread setup objects are explicitly
rejected with reprovisioning guidance, never silently widened. Executable path/hash,
agent signer, relay and optional attestation remain host-owned snapshots. Caller
executable/env/shell/MCP provisioning and identity/relay/attestation flag overrides
are rejected. Normal CLI file operations have local service-process authority;
this adapter is not an OS filesystem sandbox or a shell tool.

### Short source-driven comparison / authority choice

- Desktop `managed_agents/runtime.rs:290` selects canonical MCP; `:574` injects
  the agent private key and relay; `:708` injects optional owner attestation.
- `buzz-acp/src/lib.rs:5894` builds MCP with that agent signer/relay/attestation.
  `pool.rs:1732` adds git hints, not authoritative per-turn permission metadata.
- `buzz-dev-mcp/src/lib.rs:168` delegates its buzz personality to the existing CLI.
- `buzz-acp/src/queue.rs:1380–1407` renders ordinary reply guidance and explicitly
  permits requested channel-root posts. It does NOT mint a per-turn scope grant.
- `buzz-cli/src/commands/messages.rs:611–680` owns explicit/implicit mention
  resolution, member checks, immediate-parent/root lookup and publication.
- `buzz-cli/src/lib.rs:78–106` identifies relay/private-key/auth-tag override
  flags; the adapter rejects these while preserving normal command arguments.

Choice: reuse the CLI/runtime as destination/thread owner and relay membership /
agent authentication / optional attestation as permission authority, not a second
controller. Owner-only inbound dispatch remains unchanged. Outgoing mentions need
not be restricted to the owner when other channel participants are authorized.
No permissions are manufactured from untrusted model prompt prose. The fixture
model interprets upstream instructions only to choose tool arguments, as a model
would; that is not a host authorization decision.

## Process ownership and model evidence preserved

Upstream `buzz-agent/src/mcp.rs:758` and `buzz-dev-mcp/src/shell.rs:677` deliberately
detach process groups. Only childless shims may escape here; actual harness and
CLI launches use separately retained `spawnOwned` anchors. Stop/cancel/disconnect/
failed startup retain and await teardown, including TERM-resistant descendants.
No recovered numeric PID confers kill authority. Resource limits bound sockets,
requests, frames, calls, output and deadlines. Current per-connection request and
per-run shim/session caps still require future long-running recovery work.

Tool stdout is bounded UTF-8 and returned after anchored cleanup/drain, so reads
are useful. Stderr is withheld. A normal CLI nonzero exit becomes MCP `isError`,
not fatal conversation teardown; model can correct a failed member/argument request.
Containment/protocol errors still fail closed. Exact-model same-session acknowledgement,
model-before-prompt, malformed-tail fences and actual completed response proof remain.

B1 from independent `BROKER_REVIEW_C4F30176.md` was fixed in the predecessor:
optional config rejection is followed by exact-model re-ack BEFORE forwarding the
original error. Existing accepted/rejected optional config, failed re-ack and
model-changing rejection regressions remain green. Consumed independent `TOOL_REVIEW_023C9274.md` (workspace
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/`): no new high-confidence blocking defect,
B1 resolved through original/extended independent probes; strict/full12/12 and
independently verified installed signed publication. Its retained invariants are
host executable/env/key/relay ownership, childless shims, exact-model fencing,
distinct tool/ACP/publication evidence and bounded resources. This ordinary-CLI
delta subsequently received TOOLS_RECONNECT independent review. Long-lived request/session cap
recovery remains explicit subsequent work, not general MCP compatibility.

## Observable installed isolated outcome

Actual installed binaries (read-only, freshly rehashed):
- `/Applications/Buzz.app/Contents/MacOS/buzz-acp` SHA256
  `10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`
- `/Applications/Buzz.app/Contents/MacOS/buzz` SHA256
  `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`

`installed-conversation.test.ts` provisions fresh fixture credentials once, starts
one installed runtime and uses two authorized channels/threads, then a later owner
prompt in the first thread. All three actual CLI-published replies have canonical
hashes, valid Schnorr signatures, the SAME provisioned agent pubkey, expected
channel/parent, and explicit non-owner member recipient. Later reply references the later owner event, which itself references the original
root. The installed legacy CLI emits an immediate-parent `e` reply tag only; it
does not duplicate the root tag (unlike the current source contract). Three native `channels list` reads return useful channel data.
Non-member mentions fail without publication; a member who is not the configured
owner cannot trigger a reply. Harness model/session checks occur before every
prompt. The broker remains healthy after the later turns; owned Stop removes
harnesses, shims and resistant descendants. Test code never manufactures the agent
reply. No reconfiguration or new agent key between prompts.

One full-suite replay observed agent
`9aa6385895b9b03438af14e5ede3f947c0e3d71e2eb378b923dd571aec770a26`
and replies `591297eb4f81d62656c896e7ee4e2b391d5cf971e3790cb6675533113e7ca746`,
`8de7f8c4b4d15f6edbff7a7ad24cf9f78eb4ac70997bea118435cf197b25fef5`,
`81f42e7f3a5ed59c151d5152159b30e729eb72cad64b4dd502864ae91680bb41`.
These are disposable loopback fixture events, not production relay links.

Fixture model remains a fixture. This is installed runtime/CLI execution with
legacy kind-9 NIP-42/NIP-29 fixture evidence, NOT current production kind-40002,
real admission, provider/OAuth or Databricks evidence. Explicit hex mentions are
validated; unresolved implicit display-name mentions are NOT claimed to work.
Recovery also ran the installed CLI's `messages send --help` under empty env:
`--mention <MENTIONS>` supports hex or npub and is repeatable. Only hex was
exercised by the integration.

## Validation

Hermit Node24.15.0 / pnpm11.4.0; standalone registry-agnostic lock unchanged.
Predecessor fresh-store frozen install fetched all seven locked packages from the
approved mirror. No dependencies, global config changes or borrowed modules added.

```
. ./bin/activate-hermit
cd beehive
npm run check
BEEHIVE_REAL_BUZZ_ACP=/Applications/Buzz.app/Contents/MacOS/buzz-acp npm test
```

Recovered source strict/full **12/12** passed, followed by strict TS and complete
installed-enabled package suite **14/14** on the new reconnect candidate before
commit; diff check passes. The new run observed agent
`bf96b3237352b2038eb98b7907a5bc9a6927ce053c63b4fa4c7a6ac9ed53cb71` and replies
`94382d6dc7fb2174455f8f88d23a2eccedef9e2e6f4b390ec9e3ada278d6a88b`,
`f2842a378f033c62437ec7bb4662c19e37c8b9f490dfa8e56849ef16f23732c8`,
`3db6e28555bfc0f940bfc63d877b7157a662bdda702225fed20e77be12e485c6`.
The source candidate is the commit containing this recovery checkpoint. Self-review added legacy-scope opt-in migration,
UTF-8/drain correctness and explicit relay/attestation/env override negatives. No
repo-wide `just ci`, production operations, owner credential/profile reads or live
provider calls. Configured Logan Johnson author/committer and DCO retained; no
cryptographic signing key configured, so no invented signer/signature claim.

## Remaining product scope / ONE next executable step

**Next:** conservative same-identity two-host assignment/Move and multi-host/agent
selection. Journal and multi-tool/reconnect independent reviews are completed and
consumed above; R1 and U1 corrections have executable acceptance. The bounded
journal is not full recovery/parity completion.

README retains the full parity matrix: reconnect/ACKs/crash recovery; multiple
hosts/agents/setups and S1 two-host identity; reusable profiles/metadata/history;
key import/revocation; Restart/Move; other Desktop harness/provider/preset/relay-
mesh/compute support and final independent real UI review. None is excluded.
Production admission/protocol and named-service-user normal Databricks v2 OAuth /
live model remain unproven. The later live gate requires exact named-service-user
local auth and necessary community admission, not replacement of missing engineering.
Dedicated management TS relay remains explicit; no hidden privileged lifecycle API.
Selected-next != actual-run; stopped identity remains assigned. No remote secrets,
credential/workspace/session transfer or unreachable-host takeover. Hash/spawn
TOCTOU, hostile journals, rollback/clones, outbox pruning and stronger OS containment
remain outside evidence.

## 928F final validation/publication record

Final executable tree: strict passes (`K1_928F/final-strict.log`). Exactly one final
FULL DEFAULT concurrent installed-enabled package run naturally completed: **50/51,
1 failure, no skips/cancellations, 35.322s** (`full-installed-default.log`). All six
key-removal/import tests passed, including actual CLI K1 negatives, real Move and
hidden-input macOS PTY. Failure: `slots.test.ts:131` waiting for standby host
`Host online` via launch:50; stderr is discarded by that existing test driver.
Cause remains **unclassified**. No serial masking, rerun-to-green, timeout inflation,
assertion weakening or claim of full-suite acceptance. Prior four 110s timeouts and
other historical failures above remain independently scoped and unclassified except
for the specifically traced private-journal/TUI observation revision race.

Self-review checked common retained-state loader extraction, exact key matching,
no import authority synthesis, key input/output boundaries, atomic manifest-only
writes and stopped/lock gates. macOS PTY adapter failures and strict extraction
failure are retained alongside successful focused runs. Installed binaries were
read-only and hashed separately: buzz-acp
`10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`, buzz CLI
`147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`.
These are installed runtime + deterministic ACP/loopback fixtures, NOT live provider,
OAuth, production kind-40002 or full-product parity evidence. No dependency changes.
Source hashes and publication verification live beside the logs; the final source
commit is the commit containing this section. Required author/committer Logan
Johnson <loganj@squareup.com>, DCO, no cryptographic signing claim. No repo-wide CI,
merge or release. Remaining second-phase work and full-suite failure investigation
are explicit handoff items, not closed by this partial key-import tracer.
