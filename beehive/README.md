# Beehive (experimental first slice)

Standalone TypeScript host + relay + terminal UI. **Not Desktop parity, not
production-ready, not compatible with the current Buzz relay.** No existing
client, relay policy, credential store, or native service is changed.

A locally created owner secp256k1 identity signs encrypted commands/receipts.
Hosts retain fixed agent authority and supervise external processes. The TUI
can close while the host/runner continues. No direct-host HTTP API or controller
service exists. This package intentionally lives alongside, not inside, the
native clients: it is a separately authorized product experiment.

## Run an isolated fixture

Requires Node >=22.18 (tested with Hermit 24.15.0), POSIX process groups and
pnpm 11.4.0 for the standalone lockfile. Activate Hermit from the repository root.

```sh
cd beehive
pnpm install --ignore-workspace --frozen-lockfile --ignore-scripts
npm test
npm run check
node src/cli.ts identity /tmp/my-beehive-owner
# Prints public key; never prints the private key.
node src/cli.ts setup /tmp/my-beehive-host /tmp/my-beehive-owner/identity.json
# Choose fixture, absolute Node executable, an allowed real workspace directory,
# and this package's absolute test/runner.ts path. Confirm NEW agent key creation.
node src/cli.ts relay 19481 <printed-owner-public-key> /tmp/my-beehive-relay.json
# Separate terminal:
node src/cli.ts host /tmp/my-beehive-host ws://127.0.0.1:19481
# Separate terminal:
node src/cli.ts tui /tmp/my-beehive-owner/identity.json ws://127.0.0.1:19481
```

On Block-managed machines, the public npm registry is policy-blocked. Append
`--registry https://global.block-artifacts.com/artifactory/api/npm/square-npm/`
to the install command to use the approved mirror. No credentials or persistent
registry config are needed. The committed lockfile is registry-agnostic.

TUI: `agents`, `operations`, `reconcile`, `retry <number>`, `hosts`, `select <number or unique host-name>`, `show`, `start`, `restart`, `save`, `stop`, `quit`.
Save asks for advertised model, workspace and independent behavior profile.
The fixture has one allowed model. Named behavior profiles are authored remotely
with `profile-new`, `profiles`, `profile-edit <number>`, and `apply <number|default>`.
Useful multi-setup selection remains to build.
`show` separates selected-next and immutable actual-run snapshot. Save does not
restart. `quit` leaves the host running. Reopen TUI to inspect it; `stop` confirms
the owned process group is absent and leaves the agent assigned to this host.
Ctrl-C on **host** stops its owned fixture runner before exiting.

### Recover an UNKNOWN operation

`operations` lists numbered host/action/revision entries and historical results.
A terminal host rejection is **failed**; a matched accepted receipt is **completed**.
Neither a socket connection nor relay echo proves completion or current host state.
Use `hosts`/`show` for separately dated host inventory.

After a relay policy close (1008), unresolved work stays **UNKNOWN**, with automatic
retry disabled even after Quit/reopen. Repair policy outside the TUI, then:

1. `reconcile` reconnects and queries retained host receipts through the relay.
   It does **not** resend policy-blocked operations or prove policy is repaired.
2. `operations` shows any recovered terminal result. If still UNKNOWN and retry
   is appropriate, use `retry <number>` for the displayed operation, and confirm
   `yes` after reading the warning. This attempts the original signed encrypted
   event **once**, preserving its ID, fingerprint, body and revision preconditions.
3. A repeated denial stays blocked; another attempt requires another explicit
   action. Even a successful socket send leaves UNKNOWN until a matched host receipt.

Retry does not clear the durable automatic-retry block, cancel remote work, mint a
replacement ID, update stale revisions, or re-sign an expired/invalid event. If the
original event can never be admitted, continue reconciliation; there is no honest
terminal result to invent. The development envelope has no built-in expiry, but
external policy may still reject it permanently. Keep the retained intent. Journal
retention (1,000 operations), pruning/log-full and host-crash recovery remain unfinished.
Ordinary transient loss still has bounded automatic unchanged replay. Quit changes
only UI transport lifetime, never the pending operation or host lifecycle.

**Do not point fixture mode at real agents.** This seam deliberately passes no
agent key, OAuth context, or model to the fixture. It passes only the selected
nonsecret behavior override for external-boundary acceptance. Executable trust
is a local operator decision; remote messages cannot supply executable/args/env.
The runner must remain within its process group. Escaping descendants require
stronger OS containment before arbitrary harness support.

## Relay boundary and trust

Dedicated loopback-only WebSocket relay, not a Nostr relay, and no reserved kind
numbers. All application traffic is runtime-validated, Schnorr-signed and
AES-256-GCM-encrypted using a domain-separated owner-secret-derived key. The
relay is provisioned with only the owner public key, verifies publications and
stores ciphertext atomically before fanout. Local unauthenticated readers can
receive ciphertext and traffic metadata; this is NOT hosted private admission.
The protocol must receive independent cryptographic review before deployment.
Existing Buzz clients cannot consume these messages. Hosted compatibility needs
an independently validated private relay route/protocol decision; adding an
arbitrary Nostr kind is not an admission solution.

One dedicated owner shares signing/decryption authority with fully trusted host
installations. Host IDs route messages; they are NOT cryptographic host proof.
Agent identities are generated separately and only locally. Standby key import
attaches to an existing public genesis; only explicit experimental source-consumed Move can transfer assignment. Setup refuses an existing host directory; saved journal
bindings prevent owner/agent/host replacement from resetting admission.
An owner can still bypass software by cloning/recreating installations: safety
assumes non-cloned, non-rollback, exclusively supervised installations.

## Lifecycle claims and deliberate limits

- Exact operation ID/fingerprint and revision preconditions; durable admission
  before effects, persistent replies/outbox, idempotent replay. No exactly-once
  claim. Fixture Start means process spawned; ACP Start additionally requires probe evidence, **not Buzz conversation readiness**.
- Immutable captured selection/executable digest; executable and workspace
  prerequisites checked immediately before fixture launch. Executable replacement
  between check and spawn is outside this trusted-operator preview; full prepared
  launch must pin all setup/credential/script generations and close that gap.
- Stop uses a live TypeScript group anchor and private in-lifetime IPC channel.
  The anchor outlives the external runner, preserving group identity if its leader
  exits. Stop asks that anchor to signal its own group, retaining the living anchor
  through a 500 ms TERM grace period before ownership-safe KILL escalation, then confirms group absence;
  it never sends a kill to a recovered or possibly reused numeric ID. Quarantine
  preserves this teardown route. A lost anchor fails closed and retains the lock.
  This is POSIX in-group supervision, not containment of escaping descendants.
- Interrupted operation replies report reconciliation required. Abrupt host death
  leaves an exclusive lock: restart is blocked, not an automatic PID-based
  takeover. Local recovery tooling and crash-injection coverage remain pending.
  **Do not remove locks just because a PID seems absent.**
- Local journals are necessary admission/safety/outbox records, not a second
  remote authoring authority. Commands, configurations, receipts and inventory
  cross the relay. Journal schema validation/migration and transactional scaling
  remain pending; current journal is operator-trusted local state.
- Relay history max 10,000 envelopes, per-envelope limit, slow-reader disconnect.
  Log full closes publisher; no compaction. Outbox retained without ACK pruning;
  host auto-reconnect replays saved receipts and relay history with operation-ID
  deduplication (eight attempts/lifetime, bounded backoff). Per-event relay ACKs remain absent. Durable
  TUI pending intents/reconnect are implemented with a local encrypted journal. Lost connection means
  unknown, not stopped or success; exhausted recovery needs operator attention.
- Inventory has freshness, not liveness proof. Host starts no action based on peer
  agent presence. Only its locally bound slots are manageable; each has independent authority,
  revisions, receipts, admission and process ownership under one installation.

## Explicit initial assignment (Move prerequisite, not Move)

New local setup writes both setup and authority journal before a host can launch.
The journal pins a public genesis (owner, agent, initial host, random enrollment
ID) and a separate assigned-host field. Start/Save/Stop are denied at a standby
host even with the identical local private key. Stop and service restart retain
assignment. Startup refuses a missing journal rather than recreating authority
from a cached key. Lifecycle checks current local key/setup availability before
Start and never writes keys back; Stop does not need provider auth or a key file.

For a second, independently provisioned installation, export only the public root:

```sh
node src/cli.ts assignment-export /absolute/source-host /new/public-genesis.json
node src/cli.ts setup /new/target-host /local/owner-identity.json /local/agent-key.json /local/public-genesis.json
```

The key file is an owner-only local JSON `{ "secret": "<hex-private-key>" }`,
provisioned by the local administrator, not sent over the management relay. The
public root file is also read/written owner-only by these commands. The key must
match that root; import cannot create a second initial-authority installation.
No credential, workspace, session or provider configuration is exported. This is
not existing unmanaged-agent enrollment, nsec parsing or a key-export feature.
New-key setup remains the ordinary guided command without those optional files.

Existing pre-assignment **stopped journals** require explicit local migration:
`node src/cli.ts migrate-assignment /absolute/host-directory`. Verify there is
exactly one installation and no unmanaged execution of that identity, then confirm.
Migration preserves revision, receipts and configuration, uses the host exclusion
lock and refuses missing journals, unknown execution or an already pinned assignment.
It cannot recover journal loss, clone/rollback or retire an old installation.
A partial setup remains inert and requires local investigation, not automatic repair.

`agents` groups observations by persistent public key; `hosts` numbers host/agent
rows, shows reported assignment, and `select` routes to the exact row. A unique host
name remains a backward-compatible shortcut. Observations are not consensus; an
unreachable source never makes its standby eligible. The original assignment test uses three distinct installations. The newer
`slots.test.ts` separately proves two identities in ONE host process/connection;
these are independent agent slots, not cloned daemons.

**Experimental Move path implemented below:** destination local preflight, source
reservation/Stop, consumed grant/outbox lineage and target acceptance/fresh launch.
Preflight prerequisites and post-grant actual conversation readiness are separate gates. The root ID
is an identity binding, not a lock service or partition/rollback solution.

## Several agents, one installation

Upgrade an existing **stopped**, assignment-pinned installation explicitly, then
add an independent identity using the same locally trusted harness setup:

```sh
node src/cli.ts migrate-slots /absolute/host-directory
node src/cli.ts add-agent /absolute/host-directory
node src/cli.ts host /absolute/host-directory ws://127.0.0.1:19481
```

Both commands ask for confirmation and refuse a live installation lock. Migration
replaces only setup inventory atomically; the original journal stays byte-identical
with its assignment, revision, operation receipts and outbox. It never generates or
copies the existing key. Legacy single-slot hosts still run without silent upgrade.
Pre-assignment installations must use `migrate-assignment` first. Missing journals
and partial additions fail closed, not an invitation to re-enroll an identity.

`add-agent` creates a NEW key, or accepts local key/root files for a standby import
using the same optional-file convention as `setup`. Host-owned `default` harness,
executable/auth context and conversation setup are shared; agent keys are not part
of that reusable harness. A version-2 owner-only `setup.json` holds one installation
identity, setup inventory and per-agent key/setup references. New authority journals
live under `agents/<public-key>/journal.json`; these directories cannot run daemons.
The original slot keeps its journal in place. Maximum 32 slots per installation.
Changing local conversation setup requires every slot stopped and updates the shared
harness; `auth-info` prints the shared service-user context without signing in.

Use remote TUI `hosts` then `select <row-number>` to configure/start/stop each agent.
A host-name shortcut is rejected when ambiguous. `agents` groups persistent public
identities and labels reported assignment separately from the observing host. Each
slot retains independent selected-next/actual-run, revision, operation IDs/outbox and
admission queue. Stop X does not stop Y or change Y's receipt/revision. One host
process holds ONE installation lock, heartbeat and management connection. Reconnect
replays each slot's receipts; client reconciliation queries every unresolved
host/agent pair. Host shutdown attempts every slot even if one is uncertain, reports
failure and retains the installation lock rather than falsely declaring all stopped.

Setup/key enrollment remains local and offline; ordinary selections and lifecycle
remain remote. Multiple selectable harness inventory entries, profile CRUD, key
revocation UX and live-provider/production acceptance are still future work. Standby
X elsewhere cannot Start without a grant, even while source X is stopped/unreachable. This is trusted-installation
coordination, not physical host attestation, partition safety or exactly-once execution.

## Real target: Buzz Agent + Databricks v2

Local setup option 2 now takes **buzz-agent**, not buzz-acp. It creates fresh
owner-only `service-home` and `agent-config` directories beneath the new host
directory. Existing option-2 setups need deliberate local reprovisioning of the
runner and these paths; there is no automatic migration or credential import.
Never substitute an old Desktop configuration/cache.

```sh
node src/cli.ts auth-info /absolute/host-directory
```

This prints the exact shell-quoted executable, HOME, BUZZ_AGENT_CONFIG_DIR and
DATABRICKS_HOST context, with the host name and current OS uid. **The operator
must run that command as the same OS user that runs the host service.** It does
not perform login. Buzz Agent owns normal browser OAuth, cache, client/scopes and
headless refresh; Beehive does not broker tokens or inherit DATABRICKS_TOKEN.
No actual account/workspace is preconfigured here. Save and Stop never log in.

Start in this preview launches a bounded TypeScript ACP boundary directly to the
locally selected external executable. It negotiates ACP v1, creates one session,
reads the catalog, requires `session/set_model` to acknowledge the exact selected
`databricks-claude-haiku-4-5` **and session ID**, then requests a short greeting on
that same session/process. Only a completed `end_turn` with same-session text
produces evidence: session ID, exact model, executable SHA-256 and response hash.
Text, raw diagnostics, names/descriptions and credentials are not published.
Selected-next is independent of the running session; the process remains owned
until Stop. Auth/protocol failures do not become accepted Start.

**This is an ACP integration probe, NOT yet a conversational Buzz relay agent.**
The persistent Beehive agent key binds host authority but is not supplied to this
stdio-only probe, and this unconfigured path does not use the conversation broker. Model
acknowledgement is the trusted external executable's claim, not cryptographic
provider/model attestation. Only deterministic external ACP fixtures have passed;
no credentialed Databricks response, live account or installed binary ACP
capability negotiation was observed. Installed `/Applications/Buzz.app/Contents/
MacOS/buzz-agent --help` with a fresh isolated HOME/config returned provider-required
(exit 2); that is executable discovery, not an ACP capability test.

The narrow protocol seam is grounded in pinned main `051c3a2`:
`crates/buzz-agent/src/lib.rs` initialize (349), session/new (500–610),
session/set_model (633–682); `config.rs` BUZZ_AGENT_MODEL (642), Databricks token
optional (676), `databricks_v2` selector (939); `llm.rs` effective-model request
routing (85–147). No unmerged `BUZZ_ACP_REQUIRED_MODEL` flag is used.

**Catalog uncertainty is explicit.** `not-probed`, `reported`, `empty`, `filtered`
and `failed` are distinct, but all currently carry authentication `unverified`:
main's session/new can return a configured-model fallback after an OAuth or
network discovery failure. A reported/empty catalog cannot establish authenticated
availability or distinguish refresh-expired/denied/no-credential. That diagnostic
seam remains missing in the external protocol; do not relabel it authenticated.
An empty/filtered catalog does not prevent trying an explicitly saved exact model;
only same-session acknowledgement plus completed response establishes probe evidence.

## External conversation setup (ordinary Buzz CLI tools)

`node src/cli.ts conversation-setup /absolute/existing-host-directory` attaches
an installed **buzz-acp** and separate conversation relay to the SAME existing
agent key. Close the host first; setup uses its exclusion lock. It never reads
owner profiles, performs sign-in or regenerates an identity.

Start now uses a private per-run TypeScript ACP broker. External buzz-acp owns
Buzz authentication, membership, mentions and dispatch; its detached childless
stdio shim cannot choose executables/env/keys. The host separately owns the actual
harness process group. The proxy enforces exact-model acknowledgement at each
conversation session boundary and hashes the actual completed response, never a
separate greeting. Start waits up to 30 seconds for a real inbound conversation;
missing relay admission or local sign-in fails Start. Stop tears down all retained
runtime/harness groups and confirms shim exit. No raw text or credentials enter
management observations. This is not provider attestation or delivery proof.

Both external process-contract fixtures and the installed buzz-acp have exercised
this broker. The installed-binary test uses fresh fixture identities and an
isolated NIP-42/NIP-29 relay fixture, not a real community or provider. That binary
subscribes to legacy kind 9; this is not current Buzz relay compatibility evidence.
**An actual signed threaded reply is now observed** through the installed Buzz CLI
in that isolated test, with a deterministic ACP harness (not a live model). Local
`conversation-setup` can optionally provision a canonical Buzz CLI executable.
The fixed stdio MCP `buzz({argv, stdin})` tool passes native arguments to that CLI
under the provisioned agent identity. Use its `--help` to discover operations.
The existing CLI owns member/mention/thread semantics; no local channel, parent,
or recipient setup is required when another owner conversation arrives.
Explicit hex `--mention` supports authorized non-owner channel members too.
Implicit display-name mention behavior remains the installed CLI's contract and
has not been validated here.

The host rejects caller executable/env/MCP provisioning and relay/signer/attestation
flag overrides. There is no shell parser or HTTP/SSE MCP support. Native CLI
operations (including file operations) have local service-process authority, not
an OS sandbox. Old single-thread tool configurations require explicit rerunning of
`conversation-setup`; they are not silently broadened. MCP shims remain childless
and every actual CLI group retains its own host anchor through cleanup. Bounded
stdout reaches the model for reads; stderr stays withheld. Ordinary CLI errors are
tool errors, not a reason to destroy a healthy conversation.

Installed isolated validation covers two channels/threads and a later prompt in
the first thread, three signed replies under one agent identity, native channel
reads, non-member mention rejection and owner-only incoming dispatch. No local
reconfiguration or key generation occurs between prompts. This new surface still
has received independent tool/reconnect review; its R1 and later journal U1
recovery corrections are recorded in CHECKPOINT.md with exact evidence and limits.

Community admission/attestation and normal OAuth in the named host-service context
remain explicit local/operator actions. No live Databricks request has occurred.

## Supported / remaining parity

| Surface | This slice | Required next work |
|---|---|---|
| Local owner/key setup | New owner + separately generated agent key | nsec import, saved-owner verification, revocation/key removal UX |
| Host inventory | Real relay, multiple independent slots, shared host harness, freshness | multiple selectable setups, service install, richer reconciliation |
| Remote configuration | CAS model/workspace + reusable immutable profile publication/application; captured actual-run history | named launch configurations, metadata, setup revision pinning, retention UI |
| Lifecycle | Start/Stop/Restart and experimental source-consumed conversation Move, UI-independent host | production admission/live-provider acceptance, installed-conversation Restart acceptance |
| Buzz Agent Databricks v2 | Local setup + TypeScript ACP probe boundary | live OAuth/model proof, production protocol compatibility and catalog provenance |
| Other Buzz Agent providers | Not implemented | Anthropic, OpenAI-compatible, Databricks legacy, OpenRouter |
| Goose / Claude Code / Codex | Not implemented | harness-specific local auth and adapters/catalog/launch |
| Ten presets / custom ACP | Not implemented | explicit executable/env trust, preset-specific capability grounding |
| OpenClaw Gateway | Not implemented | external containment/environment boundary |
| Relay mesh LLM mode | Not implemented | separate transport integration, not generic provider alias |
| Compute deployment providers | Not implemented | assess `buzz-backend-*` separately from LLM providers |

None of the remaining rows is an owner-approved exclusion from the requested
full parity. Review actual executable UX/security/lifecycle before expanding.

## Experimental identity-bearing conversation Move

On two separately provisioned, trusted installations with the SAME agent key and
pinned public genesis, use `hosts`, select the source agent row, then `move` and
confirm the destination. Hosts exchange preparation/grants over the existing relay;
the TUI sends intent only. Target selection uses its locally configured allowed
model/workspace/profile. No key, workspace, credential or conversation session moves.
The source receipt means **source consumed + grant durably queued**, NOT destination
process exit/start evidence. Observe target inventory separately.

The source retains an irreversible grant chain/outbox after Stop. A destination
accepts an exact successor once before fresh Start; missing/changed prepared inputs
leave it assigned but stopped. Retry/inspect replays the grant without restoring the
source. A stopped or unreachable source does not authorize a standby by itself.
Preflight validates local key/setup, allowed selection, canonical executable hashes,
script inputs and workspace identity. Identity-free bounded `--help` runs check the
installed runtime and CLI contracts. An independently owned ACP greeting probe checks
usable local harness/model/auth prerequisites without any signer, relay or tools.
It is completely stopped before publishing preparation. Catalogs can be fallback
metadata; neither catalog nor this probe proves a **future** conversation will work.
Inventory labels preparation separately; it never becomes actual-run evidence.

After irrevocable grant the fresh target conversation must independently satisfy the
existing broker's exact requested model/session acknowledgement and completed response.
Failure leaves target assigned/stopped (or quarantined if teardown is uncertain), never
source resurrection. Restart/replayed preparation preserves the original immutable
grant binding; lost live readiness forbids automatic launch but not target acceptance.
A new Move operation is required to retry a preflight whose target lifetime ended.

`installed-move.test.ts` proves two real host processes, installed buzz-acp + Buzz CLI,
same locally provisioned signer, source exit before grant delivery/target actual spawn,
then signed target replies with correct channel/parent/recipient and same-run model
completion. It also injects actual model rejection only after grant: target stays
assigned/stopped, source cannot Start. Model/provider remain deterministic fixtures;
no actual Databricks/login/community calls or production kind-40002 proof.

**Scoped exit barrier supported:** independent `MOVE_REVIEW_97A9050A.md` source review
and grant-time runner/TERM-resistant-descendant ESRCH evidence establish the current
exclusively supervised POSIX in-group barrier. Broker-owned harness/tool groups and
childless shims retain their separate teardown checks. This is not arbitrary escaped
process containment. The historical unrecorded PID failure remains unclassified; a
new reviewer observation was empty bytes converted to PID 0, not a live descendant.
Fixture spawn now validates a positive PID and publishes it atomically. ESRCH and
bounded failure diagnostics remain, without added teardown delays or weaker assertions.
Trust requires dedicated non-cloned/non-rollback installations, exclusive supervision
and local provisioning; shared owner signatures do not attest physical hosts. No
exactly-once/partition-safe transfer, workspace/session transfer or full parity claim.

## Explicit remote Restart (incremental)

Select the assigned agent row, then enter `restart`. No operation ID or config JSON
is needed. Restart uses the existing durable relay intent/receipt journal and the
same per-slot assignment, revision and Stop-cancellation authority as Start/Move.
It captures selected-next, validates local inputs before stopping, and (for ACP)
performs a bounded identity-free model/auth prerequisite probe while the old run
remains owned. Probe teardown completes before old-run teardown and fresh launch.
This does not claim future conversation readiness or provider attestation.

Missing local setup/key or failed prerequisite preserves the existing actual run.
After old-run exit, failures leave assigned/stopped (or quarantined for uncertain
ownership), never a fallback run. An admitted Stop cancels pending Restart; queued
Save cannot mix inputs and receives a revision conflict after successful Restart.
Retry of the same operation replays its result, not another launch. Setup/credential
stores remain local, and Restart never recreates a key. Trusted local executable
replacement between final validation and spawn remains outside this preview.

Current tests exercise actual two-agent TUI Restart X with Y's journal unchanged,
fixture replay, missing setup and delayed ACP prerequisite cancellation. Named
behavior profile authoring/revisions and actual applied-instruction evidence are
now implemented by the continuation below. Installed
conversation Restart-specific acceptance remains to add; installed Move/Start tests
remain separate reusable evidence, not a claim that this whole journey is complete.

## Named behavior profiles (0aa71)

Everyday workflow in the TUI:

1. `profile-new`: enter a name and **nonsecret** behavior instructions; review and
   confirm publication. The form is a draft until confirmed. No provider/model,
   executable, workspace, key, credential or OAuth fields exist in a profile.
2. `profiles`: numbered immutable versions, parents and instructions. `profile-edit
   <number>` publishes a new child of that exact version. Concurrent edits remain
   explicit branches; neither timestamps nor arrival order choose a latest version.
3. `hosts` → `select <number>` → `apply <profile number>` selects that exact revision
   for that agent at its advertised expected revision. `save` also accepts a profile
   number (first run `profiles`) alongside allowed model/workspace. Host acceptance
   is distinct from publication. `show` names agent, host, current and selected-next.
4. `start` or explicit `restart` captures the selected snapshot. Editing/publishing
   or applying while running never restarts anything, changes actual instructions,
   touches another agent's journal, or creates/restores credentials.
5. `apply default` explicitly clears the override for selected-next only. No delete
   operation exists: historical revisions and running snapshots cannot be stranded
   or silently switched. Retention/archiving is future work, not destructive CRUD.

**Authority:** `src/profiles.ts` is the sole profile codec/lineage projection. The
existing authenticated encrypted relay publications are definition truth, scoped by
relay/owner; hosts and UI replay that same read-only projection. Content addresses
cover canonical name, explicit parent and instruction bytes. There is no mutable
host/CLI/broker profile database or new controller. The assigned host alone accepts
per-agent association changes using existing Save/CAS/receipt durability. Missing
or conflicting lineage remains incomplete, never executable. Publication completion
means the relay observed the immutable event, NOT any agent applied it. Unsent
publications use the existing encrypted client intent journal and unchanged replay.

A selection contains the validated complete profile revision, not a floating name.
The existing Start/Restart/Move preparation token covers those effective instructions.
Move preserves the source's selected public behavior while choosing target-local
launch settings; target keys/workspace/session are never copied. Later catalog
publications cannot change an asynchronous preflight or accepted historical selection.
Successful runs retain immutable full snapshots in the existing per-agent journal;
Stop/Restart cannot overwrite them. Inventory exposes the last eight run IDs/revision/
instruction hashes; actual-run carries full selected instructions plus prepared-input
hash. Legacy journals/default selections need no migration and gain no invented old
run evidence. `upstream-default` deliberately records no fabricated default-text hash.

The host passes exact captured instructions through upstream `BUZZ_AGENT_SYSTEM_PROMPT`
and installed `BUZZ_ACP_SYSTEM_PROMPT`. These are behavior inputs, never authorization
for lifecycle, membership, tools or destinations. Existing owner-only dispatch,
model-before-prompt and fixed CLI authority remain unchanged. Applied-instruction
hash covers the explicit override, **not** all upstream orientation/memory/history.
External fixture logs prove received bytes; installed conversation acceptance also
observes the marker in the actual runtime-generated ACP prompt. No live-model
obedience or production kind-40002/admission/OAuth claim follows from that evidence.

Bounds: 2 KiB UTF-8 instructions, 1,000 catalog versions, 100 parent edges, 1,000
retained successful runs per agent. Start/Restart refuse when run retention is full;
Stop remains available. Wire payload checks and a conservative Move lineage budget
refuse oversized new work before consuming source authority. No automatic pruning.
The TUI currently uses a one-line instruction form; protocol supports line breaks.
Named launch configurations, local key/setup CRUD and a small harness-specific wizard,
Desktop harness/provider/preset/custom/mesh/compute parity, final real-TUI UX/live
acceptance and independent review remain broader work, not completed by profiles.
