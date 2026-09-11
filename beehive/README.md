Instruction editing: see [Private instruction profiles](PRIVATE_PROFILES.md) for offline durable drafts, owner-private relay publication/reload and selected-next versus actual-run application. Public kind0 metadata is not included.

# Beehive (experimental first slice)

Standalone TypeScript host + relay + terminal UI. **Not Desktop parity, not
production-ready; deployed relay/provider compatibility remains unverified.** No existing
client, relay policy, credential store, or native service is changed.

A locally created owner secp256k1 identity signs encrypted commands/receipts.
Hosts retain fixed agent authority and supervise external processes. The TUI
can close while the host/runner continues. No direct-host HTTP API or controller
service exists. This package intentionally lives alongside, not inside, the
native clients: it is a separately authorized product experiment.

## Command launcher (one-time, reversible)

`bin/beehive.cjs` is a plain launcher for this checkout: it checks Node
(>= 22.18, with a short actionable error otherwise) and runs `src/cli.ts` from
its own real location, so it works from any directory, through a user
symlink, and with spaces in paths. Install once into a user-owned bin
directory already on your PATH (matching no existing command):

```sh
ln -s "$(pwd)/bin/beehive.cjs" ~/.local/bin/beehive   # adjust if your PATH differs
```

`beehive setup` (no directory) then uses the default host state folder
`~/.beehive/host`, resolved from your home and displayed once; an occupied
default is never reinitialized. Pass an explicit directory for additional
hosts. Normal configuration and private discovery need no approval files.
Undo the command with `rm ~/.local/bin/beehive` (or your equivalent link).

## Configure and join a private host

See [FIRST_DEMO.md](FIRST_DEMO.md) for the current short flow:
`beehive setup` → OWNER NPUB + RELAY → ordinary community check/join →
`beehive host --owner-present`. Setup merely configures; host start is an explicit
foreground lifecycle decision. Retained unapproved and approved identities resume
without key replacement, retargeting, file exchange or approval import.

The locally configured owner public key is the management trust root. Only that
owner's actual signed commands can manage the host. A host signs and privately
advertises its own availability, including with zero agents; this offer proves
neither owner consent nor agent placement. The owner uses
`beehive tui discover <relay> ~/.beehive/owner` without a catalog file.

Setup is configuration-only: no relay discovery, membership checks or invite prompts.
Retained setup does not open credentials. Host startup and owner management use
private NIP42/NIP59 transport directly; actual relay refusals fail as transport
errors. Host availability waits for accepted publication. Actual per-agent grants,
local genesis and retained assignment still gate Start. Owner secrets stay on the
owner computer.

Historical explicit registration tooling lives under `legacy-enrollment` and
`catalog`, not the normal setup menu. Legacy two-argument diagnostic setup below
stores a shared owner key; do not use it for normal private configuration.

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
Use `binding <local-id>` to save an advertised local binding into the selected
named candidate, then explicit `restart` to apply it. `show` lists binding IDs
and definition fingerprints. Private definitions are provisioned through `local-setup` (immutable add under
the installation lock); see the small local wizard section below. Destructive
edit/remove and full initial import unification remain deferred. Legacy configurations keep their original slot binding.
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

The loopback diagnostic protocol described below is separate from normal private
host setup. Normal host/owner management uses NIP42 plus NIP44/59 with independent
signers on the configured relay; see FIRST_DEMO.md. It neither shares the owner
secret with the host nor uses the diagnostic relay as a fallback.

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
| Local owner/key setup | New owner + separately generated agent key; deliberate local agent-key removal keeps a retained public-only slot | nsec import, saved-owner verification, remote revocation/key-removal UX |
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

## Named launch configurations and shorter first setup

`setup <new-host-directory> <identity-file>` now directly produces the shared-slot
installation. Choose the fixture or Buzz Agent Databricks v2 harness, provision its
local executable and one or two allowed workspaces, then create independent agent
identities reusing **one harness setup**. The wizard shows the intended service uid,
HOME/config directory and provider login mechanism before confirmation (no login is
performed). Each extra identity is a deliberate yes/no choice, not another provider
setup. Optional owner-only key/root file import remains standby-only. Existing hosts
still use explicit stopped `migrate-slots`/`add-agent`; this does not reset them.
No extra migration command is needed for a newly created host.

In the remote TUI, select the exact host/agent row, then:

- `config-new`: name a copy of selected-next, using the **same key and harness setup**.
  The new configuration becomes selected-next after host acceptance.
- `save`: edit the selected configuration's advertised model/workspace and profile.
- `configurations`: list the bounded, host-owned named candidates and their revisions.
- `config-select <name>`: select an existing candidate without editing it.
- `config-rename`: rename a candidate; old actual snapshots keep their old names.
- `config-remove <name>`: remove an inactive candidate. Select another first if it
  is selected. Removal never removes run history, assignment or keys.
- `show`: inspect selected-next, immutable actual run and recent historical
  configuration/model/workspace/instruction/prepared-input fingerprints.
- Explicit `restart` applies the captured selected candidate; Save/select never
  restarts. Creating/selecting a configuration never creates/imports a key.

All mutations are existing durable Save operations with agent/host/revision CAS,
receipts and unchanged-envelope retries. Configuration revisions come from the
agent journal revision, so removing/recreating a name cannot reuse an applied
revision. Maximum 32 candidates / 8 KiB combined public candidate data. Unsupported
model/workspace/profile is rejected by the owning host. One local harness setup per
slot is selectable in this increment; multiple configurations may differ in allowed
workspace or behavior. Multiple selectable harness setups and richer models/settings
remain work, not generic-provider parity.

Legacy selected-next is projected as `default` without rewriting its actual run or
inventing historical names/revisions. First accepted Save materializes the registry
in the same atomic journal persist as selected-next and the receipt. Historical
runs without a configuration tag remain explicitly legacy/unversioned data.

Standby public configuration is now remotely editable, **not execution permission**.
Start/Restart/Stop/Move remain assignment-fenced. Move requires the exact target
candidate revision and local settings while retaining source-selected public behavior.
If a target Save is accepted after preparation but before the consumed grant arrives,
the target accepts the assignment **stopped**, keeps that later selected-next, and
requires a fresh explicit Start. It never launches mixed inputs, discards the accepted
Save, strands the grant, or resurrects the source. Old prepared/grant evidence remains
immutable. Normal successful Move still captures the exact prepared candidate.

**Still unfinished:** full import/reuse wizard UX,
local setup CRUD/multiple selectable setup bindings, Goose/Claude Code/Codex/vendor
login/ACP adapters, other Buzz Agent providers, ten presets/custom ACP, mesh/compute,
production admission and normal service-user OAuth/live exact-model proof. The wizard
has working fixture and existing Buzz Agent Databricks paths; the fixture is **not**
a second Desktop harness. No new provider path or live login has been verified here.

## Deliberate local key removal (public-only slots)

`remove-agent-key <host-directory> [agent-public-key]` deliberately deletes ONE
installation's local copy of an agent key while the host is stopped (it refuses a
present installation lock — no PID or stale-lock removal — and requires a stopped
slot with no actual run). The public identity, genesis/assignment, configurations
and the complete journal/receipt/run history stay in place as a **public-only
slot**; siblings and the owner identity are untouched. Only the `agentSecret` in
the 0600 manifest becomes `null`; shared harness inventory is validated to never
carry key material (a migrated manifest that did would silently re-arm a removed
key and now fails closed).

After reopen, inventory advertises the missing-key slot (`localKey` removed) with
its retained identity and history. `start`/`restart` reject **before any spawn**
until an explicit local repair — the execution credential is an optional loader,
never regenerated or inferred. `stop`/`save`/inspect/reconnect stay
credential-neutral and truthful; ordinary save/lifecycle/recovery never recreate
the key. `add-agent` refuses to recreate the public-only identity; `remove-agent-key`
again is an explicit no-op error. `assignment-export` still exports the public
genesis for a public-only slot. `conversation-setup` and legacy key-less
installations fail closed.

Move honors the removal on both sides: a Move **to** a missing-key destination
fails its destination preflight before the source Stop, preserving the running
source; a Move **from** a public-only source still consumes its retained
management authority — the destination launches with its own local key, no
secret transfer or recreation. This is a local copy removal, not a global
cryptographic revocation: no nsec is transferred, and no remote key CRUD exists.
Local re-provisioning requires explicit local reconciliation.

### Named Move identity contract

Profiles are independent behavior revisions; a named launch configuration identifies
an exact per-agent effective snapshot. Move takes source-selected public behavior and
target-local launch settings. A `named-v1` grant binds the derived new named revision
and the exact prepared snapshot, not merely the destination's previous display label.
Preparation durably reserves `max(target revision + 1, old named revision + 1)` before
replying; cancelled preparations can leave gaps. The legacy implicit default@1 is
materialized by this same rule, rather than leaving an unnamed selection divergent
from its projected named inventory. The previous target definition stays
in immutable preparation evidence. Acceptance atomically materializes inventory and
selected-next, and actual/history retain full snapshots. A later Save gets a later
revision, is never overwritten, and forces consumed-grant acceptance to remain stopped.
Reservations alone never confer Start authority. Legacy saved grant/event identity hashes
are not changed or re-signed; new materialization requires compatible host versions.

### Explicit local key reuse for a retained public-only slot

`node src/cli.ts import-agent-key <host-directory> <agent-public-key>` is a small
local repair wizard: confirm the public identity, then paste the exact 64-hex
private key at a **hidden** prompt. Never put the private key in argv, environment,
a remote message or terminal transcript. Protected piped stdin is supported;
the key is never printed. It restores only a previously removed local copy, not
an unknown identity or an assignment. The host must be stopped and its lock free.
A valid retained journal is mandatory; wrong keys, missing/malformed authority,
non-stopped slots and already-present keys refuse without modifying state.
Standby or consumed-source import does **not** grant Start. All retained history,
public configuration, sibling keys and journals remain unchanged. Ordinary remote
Save/Start/Restart still never recreate removed keys. This is deliberate local
provisioning, not global revocation/rotation or restoration of compromised authority.

This increment does not yet integrate import/new/reuse into the initial setup
wizard, manage multiple local harness bindings, or add Goose. Those, remote A/B
selection with immutable applied inputs, and remaining Claude/Codex/preset/custom/
mesh/compute parity remain unfinished. Existing file-based standby enrollment is
unchanged; no provider login/cache access or live-provider acceptance is claimed.

## Small local binding / identity wizard

After initial `setup`, run `node src/cli.ts local-setup <host-directory>` offline.
It lists existing binding IDs/fingerprints and public identities (never keys), then
performs **one** chosen action:

- `reuse`: choose an existing identity without any mutation or key recreation;
  use the remote TUI for normal public selection/configuration/lifecycle.
- `new-agent`: choose an existing binding and confirm an independent NEW identity.
- `restore-key`: select a retained public-only identity and enter its exact key at
  the existing hidden prompt. Assignment/history stay unchanged; no standby Start
  authority is created. Unknown-identity enrollment still uses the explicit public
  genesis/file-based standby path, not this restoration action.
- `add-binding`: select a compatible existing harness definition, enter a NEW ID,
  local executable and allowed workspace (fixture additionally asks for its script).
  The existing harness mode, service-user auth context and conversation authority
  are retained. This does not add another provider/harness or authenticate anything.
  Confirming saves only the binding; reopen the wizard to create an agent using it,
  or select it remotely for an existing identity. Cancel leaves no draft writes.

Edits deliberately create NEW binding IDs/revisions; no in-place edit or removal
is advertised. Historical/current references remain valid. Each mutation takes
atomic `host.lock`, including exclusion against a competing host start; changed
source definitions reject under that lock rather than saving a stale draft.
No long-held lock during prompts. New agent journal staging remains inert until
its atomic manifest activation; interrupted staging requires local reconciliation.

Actual CLI acceptance now follows initial setup → local binding B wizard → remote
TUI A/B selection → explicit Restart B with the same identity/profile, stable A
history and unchanged sibling journal. A separate CLI journey covers new/reuse and
hidden exact-key restoration. This is a small existing-installation wizard, not a
universal provider form or complete initial-setup/import unification. Goose, Claude,
Codex, other providers, presets/custom/mesh/compute and live auth remain unfinished.

## Goose and one local entry (768f increment)

`setup <new-directory> <owner-identity-file>` now offers **3 Goose**. Configure the
installed Goose executable locally (launched as `goose acp`), its provider ID and
operator-approved compatible exact model IDs. The wizard creates a dedicated
0700 `service-home`; Goose owns its provider configuration and credentials under
that HOME, independently of Buzz Agent OAuth. `auth-info <host> [binding-id]`
prints the corresponding service-user context without login or credential reads.
No executable-found/authenticated equivalence or provider-catalog claim is made.

At initial identity confirmation choose `yes` for a NEW independent identity,
`no` to cancel, or `import-standby` for a local public genesis + hidden exact key.
Standby possession never grants Start. An existing retained public-only identity
must instead use explicit `restore-key`, not ordinary Save or re-enrollment.
`setup <existing-host>` is the same entry as `local-setup <existing-host>` and
rejects replacement owner arguments. It lists identities and immutable bindings;
choose reuse/new-agent/import-standby/restore-key/add-binding/add-goose/cancel.
Every mutation is a separate explicit confirmation. Add a compatible Goose binding
with a dedicated pre-existing service HOME; new binding IDs replace old definitions
without destructive edits or rewriting actual/history. Then remotely `binding <id>`,
choose an advertised compatible model, Save and explicitly Restart. No remote
credential/env/executable fields or new provider registry have been added.

This narrow Goose path is grounded in pinned Buzz 051c3a2's Desktop catalog and ACP
client, not a promise about every Goose version. It does **not** send Goose the Buzz
Agent-only `session/set_model` acknowledgement contract. Fresh session native model
configOptions must report the exact selected model and advertise it. Stable model
configuration responses must confirm it again; unknown/mismatch/missing evidence
fails closed. Goose profile delivery uses `_goose/unstable/session/system-prompt/set`.
Compatible models are locally approved, not discovered/authenticated automatically.
Preflight response is not a guarantee of future provider or conversation readiness.

Validation uses a **Goose-shaped TypeScript ACP fixture**, not live Goose. Actual
CLI/wizard -> remote TUI -> selected-setup ACP Restart is exercised. Separately,
actual installed buzz-acp and Buzz CLI execute through the same conversation broker
against that fixture, producing three signed replies with native model/profile
mechanics. The uninterrupted wizard-to-installed-conversation journey remains a
validation gap. Kind-9 isolated relay acceptance is not current production kind-40002.
Full Claude Code/Codex/provider/preset/custom/mesh/compute parity remains incomplete.

## Normal local conversation wizard (A661)

Initial `setup` for Goose/Buzz Agent now asks **normal (default)** versus explicit
**diagnostic ACP probe**. Normal collects canonical installed buzz-acp and Buzz CLI
paths and a trusted conversation relay, validates local launch prerequisites under
`host.lock`, and provisions conversation mode in the initial definition. There is
no separate conversion command between initial setup and remote Start. Missing
runtime/tools fail with local-install/cancel/diagnostic guidance; executable presence
is not provider authentication or relay admission. Hidden standby import retains its
original assignment and does not gain Start authority.

For an existing diagnostic agent, remotely select the desired ACP binding (including
non-default B), stop execution and shut down the host service, then run
`local-setup <installation>` and choose `normal`. It captures that agent's selected
binding, asks for a **new immutable binding ID**, and commits under the same atomic
installation lock after revalidating every stopped slot, retained authority/key and
source fingerprint. Reopen the host/TUI, select the exact host/agent and use
`binding <new-id>` (ordinary public revision CAS), then explicit Start/Restart.
Local conversion never changes any agent's selection, actual run or journal.

One fsynced manifest replacement pins installation-owned conversation runtime,
relay/tool/trust and activates the new binding together. Existing binding snapshots
and fingerprints retain their meaning; diagnostic references still opt out. Normal
binding snapshots must equal the pinned common authority, not an independently
editable per-binding authority. Existing authority is reused, never silently retargeted
or widened. Legacy `conversation-setup` refuses in-place definition rewriting and
points to explicit migration and this wizard. New/import/reuse remain key-independent
of model definitions; no key/session/credential transfer or provider login is added.

Installed acceptance uses the retained TypeScript Goose-shaped harness, actual
installed buzz-acp/Buzz CLI and isolated legacy-kind9 relay: initial normal and
selected-B conversion, TUI Start/Restart, exact profile/model, same-agent signed
cross-channel/later replies, old definition/history/Y preservation and wrong-model
preflight/replay/Stop controls. Production `host()` is in-process, not a service
subprocess deployment proof. Real Goose/provider, current-kind40002 admission,
Claude/Codex/other providers/presets/custom/mesh/compute and startup-signal cleanup
remain unfinished; see CHECKPOINT for the exact full gate, not a full-product claim.

## Claude Code: bounded API-key local binding

Initial `setup` offers **4 Claude Code** with normal conversation as default;
existing installations use `local-setup` → `add-claude`. Supply canonical installed
`claude-agent-acp`, canonical `claude` vendor CLI, dedicated service HOME, an
owner-only API-key file belonging to the host OS user, and approved exact model IDs.
The wizard never prints/relays the file contents or runs install/login. Missing
credentials must be provisioned locally. Subscription login, Bedrock/Vertex and
other Claude auth/provider modes are **not implemented** by this binding.

Bindings/keys/profiles are independent: reuse/new/import-standby and existing
normal conversion operate unchanged; selecting a new binding remotely is
selected-next only until explicit Restart. Standby still cannot Start. Remote
configuration cannot provide env/executables/credentials. Fixed installation
conversation/owner/trust/relay authority does not change with the binding.

This preview requires ACP protocol 1 and exact initialize identity
`@agentclientprotocol/claude-agent-acp` (native `_meta.systemPrompt.append` support).
`session/new.models.currentModelId` must equal the requested startup model and the
model must appear in availableModels **in that actual session**, before prompts.
Missing/mismatched reports fail closed: no native config or unstable set_model is
invented, no fallback model. Claude load/resume/optional native config are refused;
model/profile changes need a fresh session via Restart. Drift fails the run.
Catalog/key/executable presence is not authentication or provider attestation.

Source and evidence: `CLAUDE_SOURCE_6F8D.md`. Real installed buzz-acp/Buzz CLI with
source-shaped TS Claude fixtures passed normal wizard/host CLI/TUI conversations;
no real Claude/provider login or inference has been tested. Codex remains unfinished,
as do other providers/presets/custom/mesh/compute and production current-kind40002
admission/full-product acceptance. This is a coherent Claude subset, not both-harness
or full provider parity.

## Codex: conditional protocol-2 API-key subset

`setup` option **5 Codex** supports normal/default or explicit diagnostic purpose;
`local-setup` → `add-codex` adds a new immutable binding, and `normal` converts the
selected diagnostic binding to a NEW normal reference. Neither selects nor starts
it: select the binding/model/workspace remotely and explicitly Start/Restart.
Supply separately installed codex-acp and codex, an owner-only OPENAI_API_KEY file,
and dedicated service HOME/CODEX_HOME. CODEX_HOME/config.toml is TOML;
CODEX_CONFIG is the host's closed JSON model override, not a path. No installation,
login, owner-cache reading or ChatGPT subscription parity. Missing credentials,
protocol-2 native profile capability or fresh exact session model evidence fails
closed before prompting. Save/Stop are credential-neutral. See
`CODEX_SOURCE_29C7.md` for exact source, fixture/installed evidence and support limits;
older statements above about entirely unimplemented Codex are historical.

### Local preset/custom ACP setup

See [PRESET_CUSTOM_SOURCE_8D6D.md](PRESET_CUSTOM_SOURCE_8D6D.md) for the ten
source-pinned diagnostic presets, owner-only structured custom JSON and wizard
journey. Preset discovery is not authentication/readiness; arbitrary ACP adapters
are not assumed to support native model/profile contracts. Only the explicitly
opted-in Goose-native custom contract has fixture conversation acceptance here.

### Buzz Agent API-key providers (local bindings)

Normal `setup` option **8** offers Anthropic, OpenAI-compatible and OpenRouter.
Existing installations use `local-setup` → `add-buzz-provider`, reusing pinned
conversation authority without selecting or restarting any agent. Supply the
installed Buzz Agent executable, dedicated service HOME/config directory, allowed
workspace, owner-only key file, HTTPS provider base URL and operator-approved exact
models. OpenAI-compatible additionally requires explicit `auto`, `chat` or
`responses`; OpenRouter is Chat Completions only, Anthropic has no wire override.
The key variables are respectively `ANTHROPIC_API_KEY`, `OPENAI_COMPAT_API_KEY`
(**not** `OPENAI_API_KEY`), and `OPENROUTER_API_KEY`.

Endpoint, key path/value and service context remain local in immutable bindings.
Remote users choose a compatible binding/model/workspace and independent profile,
Save selected-next, then explicitly Start/Restart. No automatic install/login,
credential probe, key generation or Restart during ordinary management. Operator
model lists and keyfile presence are not authentication or actual-model proof.
Source-shaped fixture acceptance does not establish real vendor inference.
Databricks static-token setup and expanded OAuth diagnostics remain future work;
existing v2 native service-user OAuth setup remains separate (never copy owner,
Goose or Claude caches). Preset diagnostics are not provider parity.

Model/profile correction: native Buzz Agent's pinned protocol-2 contract carries
selected instructions in fresh `session/new.systemPrompt`, not just an environment
fallback. Diagnostic ACP model evidence remains guarded against same-session drift;
Restart/Move prerequisite evidence is sealed after probe teardown before old actual
teardown. Installed acceptance uses real Buzz transport/CLI with source-shaped
TypeScript provider fixtures, not live vendor authentication or inference. See
CHECKPOINT's MODEL_PROVIDER_CORRECTION entry for validation and observation limits.
