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

## Automated production seams

- `slice.test.ts`: malformed/wrong-owner/tampered encryption envelopes; real
  loopback WS relay; durable encrypted records, operation fingerprints/revisions,
  wrong-agent denial, selected-next Save, external runner Start/Stop; two actual
  TUI subprocess sessions close/reopen while host survives; host restart retains
  assignment. F2 startup lock unwind and competing live lock exclusion; F3 deliberate
  Quit does not print UNKNOWN. F1 parent exit retains descendant ownership.
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
  Tool-enabled foreign MCP and wrong-thread prompts remain rejected.
- `cancellation.test.ts` also exercises the conversation replacement path for
  admitted Stop, host close, non-authority attempts and malformed completions.
- `installed-conversation.test.ts`: opt-in with
  `BEEHIVE_REAL_BUZZ_ACP=/Applications/Buzz.app/Contents/MacOS/buzz-acp npm test`.
  Uses fresh fixture identities/dirs and an isolated NIP-42/REST/NIP-29 relay
  fixture. Verifies actual installed executable AUTH signatures, membership,
  owner mention dispatch, and exact-model same-conversation ACP completion through
  the broker. Its actual kind-9 subscription is respected. No provider is called.
  An actual signed threaded reply is required through the installed sibling `buzz`
  CLI and the fixed host MCP adapter. Verifies canonical hash/signature, same agent,
  channel/parent and sole owner p-tag; absence after Stop includes the MCP shim.
  Test code supplies the owner event and invokes the tool, never signs/publishes the
  expected agent response. This does not test a live LLM deciding to invoke tools.
- `reply-tool.test.ts`: production stdio adapter under detached childless shim;
  caller destination/mention expansion and inactive calls cannot spawn a CLI;
  fixed argv/environment; success, cancellation, disconnect, failed startup and
  CLI error all clean up the shim and any TERM-resistant in-group descendants.

Tests never open existing owner keys/profiles/cache, production services or native
client resources. With installed opt-in: 12/12 pass; without: 11 pass/1 explicit
skip. The installed fixture relay does not establish actual community admission,
current relay compatibility, provider/model attestation or general multi-thread reply delivery.

## Independent evidence and limits

Prior reviews are in workspace
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/EXECUTABLE_REVIEW_{94D1DA0E,6BD85947}.md`.
Prior c4f30176 broker independently passed strict/11 tests and installed replay in
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/BROKER_REVIEW_C4F30176.md`. Its B1 optional-config
finding is fixed with production regressions here. This new fixed-reply MCP delta
and B1 fix have not yet received independent review. The generic ACP fixture and signed/encrypted WS
management protocol are different seams; passing them does not prove current
Buzz relay auth/member/owner semantics.

No full repo `just ci`, multi-host Move, arbitrary escaped descendant containment,
strong OS sandbox, hostile journal, log-full/reconnect/ACK, rollback, host SIGKILL
recovery or real provider proof. Genuine ownership loss retains quarantine/lock;
no stale numeric PID grants cleanup authority. See CHECKPOINT.md for exact next
executable action and the wired broker, narrow fixed-thread tool, and remaining general per-turn scope gap.

## Installed executable pins

- buzz-acp: `10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`
- buzz CLI: `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`

Both are read-only `/Applications/Buzz.app/Contents/MacOS/` binaries. Their digests
are not a claim that they were built from the source base. The MCP adapter supports
only newline stdio, fixed content-only reply calls, and host-local single-thread
scope. Optional local attestation is forwarded using the normal CLI environment;
no live admission/delegation policy proof is claimed from the fixture relay.
