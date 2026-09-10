# Beehive executable checkpoint

Worktree `/Users/loganj/.buzz/REPOS/beehive-cbfd9440`, branch `beehive/cbfd9440`.
Base `051c3a270be9c73da9ab06700bcab7d5552fceaa`; prior reviewed milestone
`94d1da0e8113c27f73865233a18b1157f2ea8e4c`. This continuation is the commit
containing this checkpoint (resolve with `git rev-parse HEAD`); do not restart
repo/design discovery or create a second implementation worktree.

## Implemented now

Standalone TypeScript package; dedicated loopback real WebSocket ciphertext relay,
signed/encrypted commands, owner-only atomic storage, fixed host/agent authority,
durable revision/fingerprint admission and receipts/outbox, selected-next separate
from captured actual run, assigned host retained stopped, local setup and remote
line-oriented TUI. Existing native clients/relay/harness source untouched.

**External harness boundary:** `src/acp.ts` implements bounded newline ACP RPC to
external buzz-agent, not buzz-acp. Grounded in pinned source initialize/session/new/
session/set_model explicit {sessionId,modelId} ack, then session/prompt. Captures
local immutable launch inputs, scrubbed env with Databricks v2, dedicated HOME and
BUZZ_AGENT_CONFIG_DIR; requires exact model/session acknowledgement and completed
same-session text response. Stores response SHA-256, model, session and executable
SHA-256, never raw text/diagnostics. Host Start uses this production path; Save and
Stop do not consult auth. No unmerged BUZZ_ACP_REQUIRED_MODEL assumption.

`setup` option 2 provisions buzz-agent plus fresh service-home/agent-config dirs;
`auth-info <host-dir>` prints the exact shell-quoted local OAuth command/context,
host name and current uid, without starting login. Operator must use the host
service's OS account. No token broker, remote auth RPC, inherited provider secrets,
key import or historical cache access. Existing option-2 setup (buzz-acp) requires
explicit local reprovisioning, not automatic migration.

**Honest narrow seam:** current upstream session/new may return fallback catalog
on OAuth/discovery failure. Thus not-probed/reported/empty/filtered/failed are
explicit, but authentication stays unverified. Authenticated-empty, expired,
denied vs missing auth cannot be inferred. Catalog success is not readiness.

**Not yet a Buzz conversation agent:** this boundary keeps an ACP session alive
following a greeting probe; persistent Beehive key binds control authority but is
not supplied to stdio probe. Relay conversation/prompt bridge and actual live
Databricks execution remain gates. An external executable's model acknowledgement
is not provider attestation. Fixtures are not live proof.

## Independent feedback consumed and corrected

Read UX_ACCEPTANCE.md and EXECUTABLE_REVIEW_94D1DA0E.md in
WORK_LOGS/BEEHIVE_DESIGN_183FFAF0 (review artifacts untouched).
- F1: ordinary runner exit must not disable teardown of owned descendants.
  Added `owned.ts` + internal `supervisor.ts`: a live TypeScript group anchor
  outlives the external runner, preserving group identity. Stop is an IPC request
  to that living anchor, which signals ITS OWN group; host only polls group
  absence. No kill to a possibly reused numeric PID/group after leader exit.
  Quarantine retains verified-owned Stop/close; failed teardown retains lock.
  Unexpected anchor loss still fails closed (no PID recovery).
- F2: validate URL before lock; unwind only newly acquired startup lock after
  connect/save failure before accepting any operation. Malformed URL and refused
  connection followed by successful retry covered; competing live lock preserved.
- F3: intentional Quit prints independent host lifetime, not UNKNOWN. Unexpected
  disconnect scopes uncertainty to live reachability/unresolved requests and
  does not erase received receipts.

New candidate is NOT independently re-reviewed. S1 two-host/same-identity not
established; only single-host authority and stopped assignment tests exist.

## Commands and evidence

Activate Hermit from repo root (`. ./bin/activate-hermit`), then `cd beehive`.
`npm test`: **5/5 top-level tests pass** on Node 24.15.0; table cases include
wrong model/session, cancelled/rejected/no-response protocol, malformed JSON,
timeout/flood, catalog empty/filtered ambiguity and sanitized output. Real WS
host→external ACP fixture Start/Save/Stop, auth-directory removal before Save/Stop,
actual auth-info CLI, two TUI subprocess reopen/quit sessions, initial-connect
unwind, persistent retries/conflicts and group-anchor parent-exit regressions for
both remote Stop and host close. All test identities/processes/stores isolated.

`node node_modules/typescript/bin/tsc --noEmit`: **FAIL exit 2**, missing
@types/node/@types/ws declarations (diagnostics also cascade into implicit-any).
`npm run check` cannot find the tsc CLI because ignored borrowed dependency
symlinks have no .bin entry. No strict-check pass, lockfile or clean install claim.
Dependency investigator a2b36832 is separate; no approved dependency result has
arrived/been applied at this checkpoint. Continue integration, do not pause work
for the registry. No dependency-investigator artifacts modified.

Installed executable safe probe: fresh temp HOME/config, no credentials/env
inheritance, `/Applications/Buzz.app/Contents/MacOS/buzz-agent --help` exits 2
with BUZZ_AGENT_PROVIDER required. This proves executable presence only, not
version/capabilities/auth/model. No OAuth, live provider call or owner store read.

`git diff --check` passes. No repo-wide just ci, push or PR; no merge/release-ready
claim. Commit uses configured Logan Johnson <loganj@squareup.com> and DCO.

## Next executable step / remaining scope

Next: add a relay-driven conversation/probe operation to the retained ACP session
with run-bound request/response receipts and strict cancellation (no remote paths,
argv, env or credentials). Test it through the real host and external ACP fixture,
then have a human provision the exact host-service auth context for live-model
acceptance. Integrate approved dependency result and fix strict diagnostics as soon
as provided; request independent review of this candidate, including the anchor.

Full requested parity remains open: hosted/current-Buzz relay admission (dedicated
loopback protocol is NOT compatible), reconnect/ACKs and durable TUI pending
intents, crash recovery/strong OS containment, multi-agent/setups, profiles and
instructions/metadata, key import/revocation, Restart/Move (same key destination-local,
no unreachable takeover or secret/session/workspace transfer), all Desktop harnesses,
presets/providers, mesh and compute deployment. Agent key authority and host assignment
must remain durable; no scope exclusions are owner-approved. Executable/script/config
replacement between hashing and spawn is still trusted-local-operator territory,
not immutable file-handle pinning. No hostile-journal validation, outbox pruning,
rollback/clone protection or cryptographic deployment certification.
