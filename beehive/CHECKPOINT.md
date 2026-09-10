# Beehive executable checkpoint

Worktree `/Users/loganj/.buzz/REPOS/beehive-cbfd9440`, branch `beehive/cbfd9440`.
Base `051c3a270be9c73da9ab06700bcab7d5552fceaa`; continuation starts at
`023c9274767ef50fa0f5b37ef1883336f8be59fd`. Candidate is the commit containing
this checkpoint (`git rev-parse HEAD`). No merge/release.

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
Independent preceding broad-tool/reconnect review still in progress (evidence directory
exists; no completed report received). Existing B1/D1-D3 conclusions retained, not
reopened or treated as approval of this new journal delta.

Limits: relay URL is the community scope in this development topology, not a stable
production community identifier. Publication echo observation is session-local; terminal
host results persist. There is no relay per-event rejection/ACK protocol: policy failure
is conservatively UNKNOWN/blocked, not claimed failed; explicit guided recovery and
journal retention remain future UX work. Pre-upgrade host receipts lack fingerprints
and cannot complete new journal intents. Arbitrary disk rollback, malicious local
operator, simultaneous UI action coordination, log-full recovery and host SIGKILL
recovery are not proved. Single-operation exclusive files prevent conflicting-ID
replacement and receipt overwrite, not distributed UI serialization. Crash test is
process exit without cleanup, not machine power-loss proof. Stop-during-Start remains
covered by unchanged full-suite regressions; no claim that this new test automates the
entire interactive crash workflow.

**ONE next executable step:** independent review and real interactive TUI recovery
acceptance of this client journal, especially policy-failure UNKNOWN recovery and
receipt ordering, before expanding to the two-host same-identity assignment/Move slice.
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
source. No GitHub or NIP-34 PR has been created; repo-wide `just ci` and independent
review remain outstanding. `buzz pr open --help` requires a NIP-34 repository
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
new remote/local lifecycle endpoint. New delta requires independent review.

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
delta has NOT yet received independent review. Long-lived request/session cap
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

**Next:** independently review and exercise the local client-journal recovery
implemented above, including policy-failure UNKNOWN handling, then proceed to
two-host identity assignment/Move. The bounded journal is not full recovery/parity
completion. Independent multi-tool and reconnect review remains pending.

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
