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

Requires Node >=22.18 (tested with 26.8.1), POSIX process groups, npm dependencies.

```sh
cd beehive
npm install
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

TUI: `hosts`, `select <host-name>`, `show`, `start`, `save`, `stop`, `quit`.
Save asks for advertised model, workspace and independent behavior profile.
Only one fixture model and the `default` profile are currently supported;
profile authoring/versioning and useful multi-setup selection remain to build.
`show` separates selected-next and immutable actual-run snapshot. Save does not
restart. `quit` leaves the host running. Reopen TUI to inspect it; `stop` confirms
the owned process group is absent and leaves the agent assigned to this host.
Ctrl-C on **host** stops its owned fixture runner before exiting.

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
  claim. Successful Start means process spawned, **not model/conversation ready**.
- Immutable captured selection/executable digest; executable and workspace
  prerequisites checked immediately before fixture launch. Executable replacement
  between check and spawn is outside this trusted-operator preview; full prepared
  launch must pin all setup/credential/script generations and close that gap.
- Stop uses an in-lifetime child handle plus process group exit confirmation.
  PID alone is never used for recovery. Failed/unexpected exit quarantines.
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
  host auto-reconnect, relay ACKs and durable TUI pending intents are not yet
  implemented. Lost connection means unknown, not stopped or success.
- Inventory has freshness, not liveness proof. Host starts no action based on peer
  agent presence. Only its locally bound agent is manageable.

## Real target: Buzz Agent + Databricks v2

Local setup option 2 captures installed adapter path and HTTPS workspace URL,
creates a separate local agent identity, and advertises the target selection
`databricks-claude-haiku-4-5` as **blocked**, not discovered/model-ready. Start
fails closed because exact-model application evidence is not implemented.

Under the intended dedicated host-service OS user (not your Desktop context),
use an isolated configuration directory and the installed executable:

```sh
BUZZ_AGENT_CONFIG_DIR=<isolated-service-config-dir> \
DATABRICKS_HOST=https://<your-workspace> buzz-agent auth databricks
```

Buzz Agent owns its normal browser OAuth ceremony, workspace/client/scopes cache
and refresh. No static token is required. The actual workspace/account is a
human input; no login URL or successful authentication was observed in this
milestone. `buzz-acp models --json` is the next external catalog seam, but it
spawns an ACP session and does not prove exact-model application. Current main
has no strict `BUZZ_ACP_REQUIRED_MODEL` capability to assume. Neither owner nor
historical credential files were read/copied. An installed `buzz-agent auth
databricks --help` invocation in a scrubbed environment returned
`DATABRICKS_HOST required`; it was not a login/authentication test.

## Supported / remaining parity

| Surface | This slice | Required next work |
|---|---|---|
| Local owner/key setup | New owner + separately generated agent key | nsec import, saved-owner verification, revocation/key removal UX |
| Host inventory | Real relay, fixed authority, freshness | multiple setups/agents, service install, reconnect/reconciliation |
| Remote configuration | CAS model/workspace/default profile selection | reusable profiles, metadata, config history, setup revision pinning |
| Lifecycle | fixture Start/Stop, UI-independent host | strict real launch, Restart, host-owned irreversible Move |
| Buzz Agent Databricks v2 | Local setup + explicit blocked target | normal OAuth/capability/catalog bridge, exact-model evidence |
| Other Buzz Agent providers | Not implemented | Anthropic, OpenAI-compatible, Databricks legacy, OpenRouter |
| Goose / Claude Code / Codex | Not implemented | harness-specific local auth and adapters/catalog/launch |
| Ten presets / custom ACP | Not implemented | explicit executable/env trust, preset-specific capability grounding |
| OpenClaw Gateway | Not implemented | external containment/environment boundary |
| Relay mesh LLM mode | Not implemented | separate transport integration, not generic provider alias |
| Compute deployment providers | Not implemented | assess `buzz-backend-*` separately from LLM providers |

None of the remaining rows is an owner-approved exclusion from the requested
full parity. Review actual executable UX/security/lifecycle before expanding.
