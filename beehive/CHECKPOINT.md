# Beehive executable checkpoint

Worktree `/Users/loganj/.buzz/REPOS/beehive-cbfd9440`, branch `beehive/cbfd9440`.
Base `051c3a270be9c73da9ab06700bcab7d5552fceaa`; this continuation starts at
`9ff9c6cf16f8c5d7cae0b95e4c01cb3820ff833e`. Candidate is the commit containing
this checkpoint (`git rev-parse HEAD`). No push/PR/merge/release.

## Implemented conversation boundary — not full delivery

**The host-owned TypeScript ACP broker is implemented and host Start is wired.**
`conversation-setup` still attaches to the existing persistent agent identity,
never regenerating keys or initiating OAuth. External buzz-acp receives that key,
explicit owner, separate conversation relay, optional local attestation and model.
It retains all Buzz authentication/member/mention/dispatch/reply/tool semantics.
There is no second Buzz chat runtime or remotely callable host API.

`src/broker.ts` opens a fresh private temporary directory/Unix socket with a random
per-run capability. buzz-acp launches `acp-shim.ts` with that capability; the shim
is childless and only transports stdio. The actual harness is launched by the
host via existing `spawnOwned`, NOT by the escaping upstream process group.
Capabilities cannot cross runs or supply executable/argv/env/key choices. Session
workspace is fixed; nonempty MCP provisioning is rejected (see delivery gap).
Socket authentication deadlines, connection/process/session/request/frame bounds
and write-pressure failure prevent unbounded buffering. No raw diagnostics,
response text, keys, attestation or capability enter management observations.

The proxy forwards bidirectional requests/responses/notifications, including
cancellation, reverse client RPC and session/new/load/resume. Before releasing a
created/loaded/resumed session, it injects `session/set_model` and requires exact
session/model response. It rejects conflicting/rejected/unconfirmed model changes,
including model config choices, and re-verifies after config changes. Prompts
require a confirmed session; evidence hashes actual same-session response chunks
and requires uncancelled `end_turn`. Evidence identifies agent pubkey, harness and
external runtime executable hashes, session and model. This is trusted-executable
protocol evidence, NOT provider attestation or relay delivery confirmation.

Start waits up to 30 seconds for an actual inbound conversation and completed
response; it never manufactures a greeting on this path. Save stays selected-next;
Stop/config remain credential-neutral. Missing admission/sign-in yields a bounded
Start failure, not a running agent claim. Generic non-conversation setups still
run the explicitly labelled old anonymous ACP probe.

## Owned teardown and review regressions

Stop/cancel/failed Start closes sockets (escaping childless shims exit on EOF),
then stops runtime and every harness through retained live group anchors. Shim
PID checks are absence observations only, NEVER kill authority. Failure to confirm
any group/shim exit fails closed. Existing supervisor retains ownership through
500ms TERM -> own-group KILL, including host IPC loss; numeric journal PIDs grant
no takeover/kill authority. Arbitrary executables that deliberately escape their
owned group remain outside this POSIX containment claim.

D1 resistant descendants remain tested. D2 replacement host conversation path now
shares exact-authority/revision/body-admitted cancellation outside the mutation
queue; wrong agent/revision/body cannot cancel. Host close interrupts Start. D3
coalesced completion plus malformed tail cannot commit readiness. These regressions
are exercised for both old probe and new conversation broker paths. No independent
review of this delta has occurred; prior independent frozen-9ff9c6cf work was not
used as approval of this new implementation.

## Actual executable evidence and precise remaining gap

Automated host -> external runtime fixture -> detached/process_group(0)-style
shim -> owned ACP harness passes using persistent identity, exact model, same
conversation completion, bidirectional RPC, session loading, cancellation and
complete runtime/shim/harness/TERM-resistant-descendant absence after Stop. The
external runtime fixture models the process/ACP contract, not a Buzz relay.

Also exercised installed `/Applications/Buzz.app/Contents/MacOS/buzz-acp`, SHA256
`10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`, against an
isolated NIP-42/REST/NIP-29 fixture relay and the owned ACP fixture harness. It
signed verified AUTH with a fresh fixture agent key, discovered fixture membership,
subscribed, consumed a signed owner mention, and completed the exact-model same
conversation through the production broker. **Installed executable subscribes to
kind 9 (plus 46010/40007), not kind 40002**; the fixture follows its actual kind 9
subscription. This is evidence for that installed binary, not current Buzz relay
compatibility/admission. Test never touches owner keys/cache/profile or providers.

**No signed threaded response publication was observed.** ACP text completion is
not automatic Buzz message delivery here. The current launch plan explicitly sets
`BUZZ_ACP_MCP_COMMAND=''`; no locally authorized MCP/tool executable plan is
provisioned. The broker deliberately rejects shim-supplied MCP commands/env rather
than permitting arbitrary executable injection. Tool-backed conversation delivery
therefore remains a concrete required engineering step, not just an OAuth gate.
Do not claim the whole conversation/delivery slice or owner build is complete.

## Validation / dependencies / next action

Hermit Node24.15.0 / pinned pnpm11.4.0. Standalone registry-agnostic lockfile unchanged;
no new dependencies or borrowed modules. Approved install route remains:

```
. ./bin/activate-hermit
cd beehive
pnpm install --ignore-workspace --frozen-lockfile --ignore-scripts \
  --registry https://global.block-artifacts.com/artifactory/api/npm/square-npm/
npm run check
BEEHIVE_REAL_BUZZ_ACP=/Applications/Buzz.app/Contents/MacOS/buzz-acp npm test
```

Full suite with installed opt-in: **11/11 top-level tests pass**. Without opt-in:
10 pass, installed test explicitly skipped. Strict TS and `git diff --check` pass.
No repo-wide `just ci`, production service, owner OAuth or live provider call.
Commit uses configured Logan Johnson <loganj@squareup.com> author/committer/DCO,
without persistent config/signing changes. No independent broker review yet.

**ONE next executable action:** implement a locally provisioned, fixed-authority
MCP/Buzz-tool launch plan (including service-local tool executable discovery and
owned teardown), then extend the installed isolated-relay test to require an actual
signed threaded agent reply. Do not loosen the broker to accept shim executable/env
provisioning or invent an alternate Buzz dispatcher. Request independent broker
review against this usable artifact before broadening the launch surface.

Still open: actual community admission/attestation and named-service-user normal
Databricks v2 OAuth/live `databricks-claude-haiku-4-5` proof; catalog auth ambiguity;
reconnect/ACKs and durable TUI intents; crash recovery/strong OS containment;
multiple hosts/agents/setups, profiles, key import/revocation, Restart/Move; remaining
Desktop harnesses/presets/providers/mesh/compute. S1 two-host identity unproven.
Selected-next != actual-run; stopped identity remains assigned. Move must never
transfer credentials/workspace/session or permit unreachable takeover. Hash/spawn
TOCTOU, hostile journals, rollback/clones and outbox pruning remain outside evidence.
