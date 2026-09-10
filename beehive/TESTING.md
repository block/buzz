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
