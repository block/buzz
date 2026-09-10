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

TUI: `operations`, `reconcile`, `retry <number>`, `hosts`, `select <host-name>`, `show`, `start`, `save`, `stop`, `quit`.
Save asks for advertised model, workspace and independent behavior profile.
Only one fixture model and the `default` profile are currently supported;
profile authoring/versioning and useful multi-setup selection remain to build.
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
agent key, OAuth context, model, or instructions to the fixture. Executable trust
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
Agent identities are generated separately and only locally. Key import and
Move are disabled. Setup refuses an existing host directory; saved journal
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
  agent presence. Only its locally bound agent is manageable.

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
| Host inventory | Real relay, fixed authority, freshness | multiple setups/agents, service install, reconnect/reconciliation |
| Remote configuration | CAS model/workspace/default profile selection | reusable profiles, metadata, config history, setup revision pinning |
| Lifecycle | fixture Start/Stop, UI-independent host | strict real launch, Restart, host-owned irreversible Move |
| Buzz Agent Databricks v2 | Local setup + TypeScript ACP probe boundary | live OAuth/model proof, production protocol compatibility and catalog provenance |
| Other Buzz Agent providers | Not implemented | Anthropic, OpenAI-compatible, Databricks legacy, OpenRouter |
| Goose / Claude Code / Codex | Not implemented | harness-specific local auth and adapters/catalog/launch |
| Ten presets / custom ACP | Not implemented | explicit executable/env trust, preset-specific capability grounding |
| OpenClaw Gateway | Not implemented | external containment/environment boundary |
| Relay mesh LLM mode | Not implemented | separate transport integration, not generic provider alias |
| Compute deployment providers | Not implemented | assess `buzz-backend-*` separately from LLM providers |

None of the remaining rows is an owner-approved exclusion from the requested
full parity. Review actual executable UX/security/lifecycle before expanding.
