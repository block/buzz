# Beehive executable checkpoint

Worktree `/Users/loganj/.buzz/REPOS/beehive-cbfd9440`, branch `beehive/cbfd9440`.
Base `051c3a270be9c73da9ab06700bcab7d5552fceaa`; this continuation starts at
`c4f30176765bdedd075e9634f33e4d4576ce3619`. Candidate is the commit containing
this checkpoint (`git rev-parse HEAD`). No push/PR/merge/release.

## Implemented: actual signed reply in the isolated installed-binary path

Host -> existing installed buzz-acp -> escaping childless ACP shim -> separately
host-owned ACP fixture harness -> escaping childless MCP shim -> host-resident
TypeScript MCP adapter -> separately host-owned **actual installed Buzz CLI**.
The CLI, not Beehive/test publisher code, resolves members/thread and signs/publishes
an actual reply. The isolated relay verifies canonical event hash, Schnorr signature,
persistent fixture agent pubkey, channel, parent and sole explicit owner recipient.
This is legacy kind-9 fixture evidence, NOT current production kind-40002 compatibility,
real community admission, live Databricks/provider evidence or full product completion.

`conversation-setup` optionally provisions a canonical installed Buzz CLI executable
and a **fixed single-thread channel/parent/owner-recipient grant**. Existing identity,
provider context and assignment are preserved. No remote executable/env/key/scope
policy is accepted. The local configuration is frozen at launch; the ACP caller must
still supply EMPTY MCP provisioning. The broker substitutes only its own fixed
stdio MCP descriptor, and rejects foreign workspace/executable/env/MCP requests.

`src/reply-tool.ts` exposes only `buzz_reply({content})`, during a model-confirmed
active prompt. No shell, arbitrary CLI argv, file upload or caller-selected destination.
CLI invocation is fixed `messages send --channel ... --reply-to ... --mention ...
--content -`; content goes through stdin. Mention syntax is conservatively rejected
so implicit CLI mention resolution cannot widen the sole provisioned recipient.
Host-local signer/relay/optional attestation override all context. HOME is isolated;
no owner cache/profile or ambient env inheritance. Raw CLI output is bounded and
withheld from management/tool diagnostics. MCP is newline stdio only, not HTTP/SSE.

**Scope limitation is intentional and explicit:** the tool does not discover dynamic
per-turn authority. It only replies into the locally provisioned thread, with an
additional fail-closed check that the upstream prompt contains that channel and
`--reply-to` instruction. Text parsing cannot enlarge the local grant and is not
signed-event provenance. General automatic multi-thread routing still needs a real
request-local scope interface; do not market this one-thread provisioning UX as
Desktop parity. A prompt for another thread fails rather than silently replying there.

## Source-grounded ownership and teardown

Bounded read at base source:
- `buzz-acp/src/lib.rs:5894` build_mcp_servers derives stdio executable and signer/relay/
  attestation env; `pool.rs:1732` adds git-origin hints, not authoritative reply scope.
- `buzz-acp/src/queue.rs:1383` and following render ordinary reply instructions into
  prompts; the existing CLI owns actual member/thread resolution and signed publication
  (`buzz-cli/src/commands/messages.rs:611` and following).
- **Two deliberate process-group escapes** preclude simply enabling dev MCP:
  `buzz-agent/src/mcp.rs:758` detaches its MCP subprocess, and
  `buzz-dev-mcp/src/shell.rs:677` detaches tool shell processes. Directly forwarding
  dev-mcp would lose host ownership of shell groups on abrupt parent death.

Only the existing childless stdio shim can escape here. MCP adapter lives in the
host; every actual CLI invocation uses `spawnOwned`, with retained live group anchor.
Supervisor now forwards only numeric runner exit code so CLI failure is not confused
with successful publication. TERM-resistant in-group descendants are killed through
the living anchor; PID observations never grant kill authority. Stop, cancellation,
failed startup, CLI failure and MCP disconnect retain/await teardown promises.
Broker failure starts tool cleanup immediately and final Stop checks the same promise.
Connections, IDs, frames, calls, stdout/stderr bytes and deadlines are bounded.
Host IPC loss continues to use the existing anchored group cleanup. Arbitrary trusted
executables that deliberately escape groups remain outside this POSIX containment claim.

## Review reconciled: B1 optional-setting rejection

Consumed independent `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/BROKER_REVIEW_C4F30176.md`
(pinned c4f30176; independent strict + 11/11 and installed replay). It found one
medium B1: optional non-model config rejection cleared model eligibility permanently.
Upstream optional effort/permission rejection intentionally falls back rather than
aborting (`pool.rs:1880–1930`, `1973–2035`). Broker now conservatively re-acknowledges
the exact session/model BEFORE forwarding the original optional-setting error.
It does not assume the error had no side effects. Rejected model-setting changes
remain fail-closed. Production broker tests cover accepted optional setting,
rejected optional setting then successful prompt, failed re-ack and rejected model
config sibling. Existing wrong-model/conflict/cancellation/malformed-tail guards remain.
New MCP code and this B1 fix have NOT received independent delta review yet.

## Actual executable evidence

Installed `/Applications/Buzz.app/Contents/MacOS/buzz-acp` SHA256:
`10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`.
Installed `/Applications/Buzz.app/Contents/MacOS/buzz` SHA256:
`147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`.
Binary digests are independent of the source base; neither binary is rebuilt/modified.

One isolated run verified NIP-42, two subscriptions, exact-model same-session ACP
completion, and signed threaded publication:
- reply `47f814433a621434fc76c8d282f46e3d4c2311081823def7d4aabde281a9dfff`
- fixture agent `3daf86eecb494ae8db349ea57ca2551c55c02eb3b2b923d54bd653000dbf5522`
- parent `d5e3633b90efcc7d76f66a1e1f0079c6782c1287e4c7b9aae640414449f3b9be`
- channel `aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee`

Fixture keys/events change each run and the loopback relay is removed afterward.
This is reproducible test evidence, not an event on the production relay. Harness,
MCP shim and resistant descendant PID files are checked absent after Stop. Tests
never manufacture the expected agent reply; the fixture merely calls the real tool,
as an LLM would. No provider, OAuth or real-owner credential access occurs.

## Validation / next action

Hermit Node24.15.0 / pnpm11.4.0. Fresh node_modules and fresh store frozen install
fetched all seven locked packages from the approved mirror. No new dependencies,
borrowed modules, global config changes or lockfile delta.

```
. ./bin/activate-hermit
cd beehive
pnpm install --ignore-workspace --frozen-lockfile --ignore-scripts \
  --registry https://global.block-artifacts.com/artifactory/api/npm/square-npm/
npm run check
BEEHIVE_REAL_BUZZ_ACP=/Applications/Buzz.app/Contents/MacOS/buzz-acp npm test
```

Full package suite with installed opt-in: **12/12 pass** (including additional
B1/tool authority/cleanup cases within top-level tests); strict TS and diff check
pass. Exact candidate is re-run before commit. No repo-wide just ci, production
operations, owner OAuth, live provider or new independent MCP review.

**ONE next executable step:** independently review/replay this pinned fixed-reply
MCP delta and B1 regression, including the installed isolated signed-reply test,
before expanding to automatically provisioned per-turn scopes.

Still open: actual community admission/attestation and named-service-user normal
Databricks v2 OAuth/live model proof; current Buzz protocol compatibility; catalog
auth ambiguity; general per-turn reply scope (not prompt parsing); reconnect/ACKs,
durable TUI intents and crash recovery; multiple hosts/agents/setups; profiles;
key import/revocation; Restart/Move; remaining Desktop harnesses/presets/providers/
relay-mesh/compute. S1 two-host identity unproven. Selected-next != actual-run;
stopped identity remains assigned. No remote secrets, credential/workspace/session
transfer or unreachable-host takeover. Hash/spawn TOCTOU, hostile journals,
rollback/clones, outbox pruning and stronger OS containment remain outside evidence.
