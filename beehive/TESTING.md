# Package validation

`npm test` runs Node's real TypeScript test runner. Test identities and stores
are fresh temporary owner-only resources. No production services or provider
credentials are required. `npm run check` performs strict TypeScript checking.

Automated evidence in `test/slice.test.ts`:
- malformed/wrong-owner/tampered envelope rejection;
- actual loopback WebSocket relay persistence and encrypted payload checks;
- external deterministic TypeScript runner Start/Stop;
- wrong-agent authority denial, operation fingerprint conflict, stale revision;
- selected-next Save while a run continues;
- two actual CLI TUI subprocess sessions, each showing running actual state and
  quitting while the host survives;
- relay client close/reopen, duplicate retry, clean host close/restart with
  assignment/revision retained.

Fixture is not an ACP agent or OAuth/model test. Crash-safe multi-host Move,
unknown descendant containment, relay outages/full log, host SIGKILL, hostile
local journal modifications and production private admission are not proven.

Current environment: registry.npmjs.org requests are redirected to Block's
Cloudflare Dependency Confusion block page (HTML rather than JSON). Offline npm
has no package metadata cache. Local tests use symlinks to already-installed
public JS packages from
`~/.local/lib/janet/a8d6cdb-darwin-arm64/package/candidate/node_modules/`
(`@noble/curves` 1.2.0, `ws` 8.21.0, TypeScript 5.9.3); no private Janet data is
used and nothing there was modified. Symlinks are ignored, not committed.
`@types/node` / `@types/ws` are not installed locally; typecheck currently fails
with missing declaration diagnostics. This is not a passing typecheck or a
reproducible clean install. Restore approved dependency access, generate a lock
file, run strict check and package tests before proposing merge/production use.
No policy bypass or alternate registry workaround was attempted.

## ACP and independent-review regressions (continuation)

`npm test` now runs five top-level tests (many table cases) under Hermit Node
24.15.0. `acp.test.ts` launches `acp-fixture.ts` through the actual TypeScript
supervisor and ACP boundary. Cases cover exact session/model acknowledgement,
response hashing, empty/filtered catalog provenance, wrong model/session, rejected
operation (secret-bearing error withheld), cancelled turn, malformed JSON, timeout
and output flood. The host integration uses real encrypted WS commands and proves
Save → Start → same-session fixture evidence → Save → remove isolated auth directory
→ Stop, retaining stopped host assignment. No fixtures call an LLM or read a cache.

The review at pinned 94d1da0e is in the workspace's
WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/EXECUTABLE_REVIEW_94D1DA0E.md. Its F1/F2/F3 are
addressed with production regressions in slice.test.ts:
- orphan-fixture.ts exits its runner but leaves a non-escaping descendant; a live
  group anchor must remain, and both remote Stop and host close kill the group;
- malformed relay URL and initial connection refusal, then successful startup;
  second-live-host lock rejection remains tested;
- two actual TUI sessions intentionally quit without printing UNKNOWN.

The new anchor pins group identity while the external runner is gone. Cleanup is
an IPC request to that living anchor, which signals its own group. No negative-PID
kill from a stale parent child handle is used. Group absence polling is read-only.
Unexpected loss of the anchor still requires local reconciliation and is not a
claim of general OS containment or crash recovery. These fixes and the ACP slice
need independent review on their new head; the old review is not approval of them.
