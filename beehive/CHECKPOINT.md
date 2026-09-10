# Beehive executable checkpoint

Worktree `/Users/loganj/.buzz/REPOS/beehive-cbfd9440`, branch `beehive/cbfd9440`.
Base `051c3a270be9c73da9ab06700bcab7d5552fceaa`; previous reviewed milestone
`6bd859479503832a331cb9f8eb0a495082c8f9d8`. This continuation is the commit
containing this checkpoint (`git rev-parse HEAD`). No push/PR/merge/release.

## Outcome and exact remaining critical gap

**The requested identity-bearing relay conversation is NOT delivered.** The
existing executable path is still a retained anonymous ACP greeting probe.
Do not call its evidence a Buzz conversation, provider attestation or live-model
proof. New `conversation-setup <existing-host-dir>` provisions the external
buzz-acp contract onto the SAME existing key, atomically under the host exclusion
lock. It does not create a key, initiate auth or connect to any conversation relay.
Configured conversation Start fails BEFORE spawning or committing actual-run state.
Inventory and CLI explicitly say blocked; Save/Stop remain credential-neutral.

`src/conversation.ts` validates and snapshots the external executable, selected
Buzz Agent launch, conversation relay, exact existing agent secret, owner pubkey,
optional host-local opaque NIP-OA tag, exact model, owner-only inbound gate and
local default permission mode. Env is allowlisted; identity and relay are
explicitly authoritative. No owner secret, arbitrary env or remote paths/argv are
accepted. Upstream comma-delimited args reject lossy comma/NUL/empty elements.
Remote summary contains no credential/executable path. This is a tested launch
PLAN, not a launched identity-bearing session. CLI currently asks no attestation;
community operator must admit the public key and owner to intended channels, or
locally provision a valid NIP-OA tag. Neither that action nor live OAuth occurred.

## Source-grounded runtime decision (not a second agent runtime)

Pinned upstream `051c3a2`:
- `desktop/src-tauri/src/managed_agents/runtime.rs::spawn_agent_child` passes
  BUZZ_PRIVATE_KEY, BUZZ_RELAY_URL, BUZZ_AUTH_TAG, external agent command/args,
  model/provider/effort. No BUZZ_ACP_REQUIRED_MODEL contract exists here.
- `crates/buzz-acp/src/config.rs::CliArgs` defines these external env/CLI inputs.
- `crates/buzz-acp/src/lib.rs::resolve_agent_owner` verifies NIP-OA against the
  agent pubkey or falls back to explicit owner. Startup uses HarnessRelay::connect
  with that key/auth tag, then the actual inbound author/member gate.
- `crates/buzz-acp/src/pool.rs::apply_model_switch` treats application rejection
  as Rejected and proceeds with the default. Model env alone is insufficient.
- **Critical containment seam:** `crates/buzz-acp/src/acp.rs::AcpClient::spawn`
  calls `cmd.process_group(0)` on Unix. Replacing the probe with buzz-acp under
  the current outer anchor would let the ACP subtree escape owned Stop (F1).

Reuse existing buzz-acp for all Nostr/owner/membership/mention/reply/delivery
semantics. Do not invent a chat protocol or second Buzz dispatcher. Smallest
coherent next implementation: host-owned TypeScript ACP stdio broker/proxy so the
actual adapter is spawned under a host-retained live anchor, not as an escaping
buzz-acp descendant; the buzz-acp-launched TS shim only forwards stdio. Bound and
close every shim/broker session on Stop, retain adapter ownership after parent
exit, enforce session/model acknowledgement, and capture prompt completion on
THAT identity-bearing conversation session. This broker is NOT implemented yet.
The current guard deliberately prevents unsafe direct launch in the meantime.

Management transport remains the dedicated loopback TypeScript ciphertext WS
protocol, NOT a Nostr/Buzz conversation relay. No existing clients or Rust changed.

## Independent review integrated

Read `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/EXECUTABLE_REVIEW_6BD85947.md`.
Earlier F1/F2/F3 remain covered. This continuation addresses its D1–D3:
- D1: live supervisor ignores its own TERM, remains through a 500 ms graceful
  interval, then sends KILL to its OWN still-owned group. Host only polls absence;
  transient EPERM is not success and is retried within the same bounded wait.
  IPC disconnect/send failure invokes the same cleanup rather than killing the
  anchor prematurely. Resistant orphan descendants covered through remote Stop,
  host close and parent IPC loss. Genuine anchor loss still fails closed.
- D2: exact-authority/current-revision/body-valid Stop signals ACP cancellation
  outside the serialized mutation queue. Close cancels before waiting. Cancelled
  Start fails; queued Stop retains ordinary receipt/admission handling. Delayed
  responses cannot commit evidence after failure. Wrong agent/revision/body does
  not cancel. No blanket stale-revision bypass or recovered-PID kill added.
- D3: request promise commit and evidence commit recheck session health; host
  rechecks before running. Coalesced completed-response + malformed-tail regression
  fails at both ACP and host production seams, rather than accepting then waiting
  for heartbeat quarantine.

This candidate has NOT received independent delta re-review. Report fixtures
remain independent; no review artifacts modified.

## Dependency route and validation

Read and applied `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/DEPENDENCY_INSTALLATION.md`.
Public npm registry is host-policy blocked; approved Block mirror is the supported
route, not a policy bypass. Borrowed ignored modules removed. A standalone
registry-agnostic `pnpm-lock.yaml` now pins all seven packages with integrity.
No global/user npm/Git config or machine-path dependency committed. Use:

```
. ./bin/activate-hermit
cd beehive
pnpm install --ignore-workspace --frozen-lockfile --ignore-scripts \
  --registry https://global.block-artifacts.com/artifactory/api/npm/square-npm/
npm run check
npm test
```

Fresh empty node_modules + fresh temporary pnpm store fetched all seven packages
successfully on pnpm 11.4.0 / Node 24.15.0. Strict TypeScript check passes;
**npm test: 9/9 top-level tests pass** (13.2 seconds). Tests include original
ACP/WS/TUI regressions, D1–D3, and blocked conversation setup/launch contract.
`git diff --check` passes. Local commit uses configured Logan Johnson
<loganj@squareup.com> author/committer and DCO; no persistent Git config edits.
No real Databricks OAuth, model call, conversation relay admission or external
buzz-acp network session was exercised. Generic ACP fixtures are NOT Buzz relay
parity. No repo-wide `just ci` or production readiness claim.

## Next executable action / full scope

**Implement the host-owned ACP stdio broker described above, then remove the
conversation Start gate only when adapter/shim teardown and exact-model real
conversation correlation pass tests.** Do not spend another run rediscovering
upstream routing. Preserve the provisioned agent key and the two-relay distinction.

Still open: actual Buzz-relay conversation/owner admission, live provider proof,
catalog auth ambiguity, reconnect/ACKs and durable TUI intents, crash recovery and
strong OS containment, multiple agents/hosts/setups, profiles/instructions/metadata,
key import/revocation, Restart/Move, other Desktop harnesses/presets/providers,
relay-mesh and compute. S1 two-host same identity is unproven. Source setup is one
user-facing object but initial key setup is still coupled to first harness setup;
conversation attachment now independently preserves keys. No remaining scope row
is an owner-approved exclusion. Hash-to-spawn TOCTOU, hostile journals, rollback/
clone protection and outbox pruning remain explicitly outside current evidence.
