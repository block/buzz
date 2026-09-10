# First-demo finalization — recovered state and remaining delta

## Recovered repository state (2026-09-10)

Worktree `REPOS/beehive-demo-transport-473233b8`, branch
`beehive/demo-transport-473233b8`, approved origin `https://github.com/block/buzz.git`.
Working tree clean. Local HEAD `ac32ae0d2`
(`ac32ae0d248f61aaec1e8730b98544a93ccfb023`, "integrate private public-CLI Start
reply Stop with fresh membership"). Remote branch head at recovery:
`6caa4150d` — five local commits not yet published:
`3dbf9aaaa` (owner-public immutable bindings and local wizard),
`1315af75c` (credential-neutral assignment/auth guidance),
`ac781fd95` (direct relay membership over relay HTTP contract; module fc62c755),
`7176028b7` (module correction 30fffd4e: bounded chunked body acquisition and
signer settling), `ac32ae0d2` (main public command-path integration + FIRST_DEMO.md).

## Combined DEFAULT concurrent installed-enabled run — actual result

Owned runner PID 59155 (evidence directory
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/CLI_INTEGRATION_5A40/`, script `run-candidate.sh`,
pin `v24.15.0` + `ac32ae0d2...` + clean status) **completed naturally**; it was not
killed, restarted or duplicated during recovery. Actual recorded outcome:

- Strict `tsc --noEmit`: exit 0.
- Full default concurrent installed-enabled suite (`node --test test/*.test.ts`,
  `BEEHIVE_REAL_BUZZ_ACP=/Applications/Buzz.app/Contents/MacOS/buzz-acp`):
  **156/157 pass, 1 fail, natural duration 318624.507541 ms** (~318.6 s).
  The single failure is the previously known compatibility assertion in
  `test/enrollment-input.test.ts:35` ("actual standalone CLI private pairing,
  existing-owner approval and bootstrap return stay offline"):
  `assert.ok(imported.includes('Pending/no network'))`.
- Run exited 1 (`full-default-installed.exit`, `complete.exit`); source and
  native before/after sha256 sets identical (no file drift during the run).

## Known assertion diagnosis (actual behavior inspected)

Offline import remains offline: `enrollHostIdentity` writes the signed approval
locally only, and `requireHostEnrollment` still reports
"relay admission pending". The wording-only regression is in commit `ac32ae0d2`:
the import completion line replaced
`Host registered. Pending/no network: narrow relay admission is not implemented...`
with guidance that no longer contains the literal `Pending/no network`, while the
old test still asserts that phrase. No behavior, network use or test change is
required; restoring clear truthful `Pending/no network` guidance in the import
output is the compatibility fix. (The removed "narrow relay admission is not
implemented" claim is outdated — direct relay membership is implemented and
verified — and stays removed.)

## Remaining delta for this finalization

1. Restore `Pending/no network` wording in the offline import completion message.
2. Strict + focused offline-enrollment test after the wording-only repair; no full
   rerun solely for wording.
3. Commit and publish the five verified local commits plus this delta to
   `beehive/demo-transport-473233b8`.

## Delta result (wording-only repair, updated at test boundary)

Import completion line now reads: `Host registered offline. Pending/no network:
relay admission remains pending. Host startup requires an already-enrolled direct
relay member and a fresh live row check. No agent authorized or started.`
Behavior unchanged: import stays offline, admission stays pending; the outdated
"narrow relay admission is not implemented" claim stays removed.

- Strict `tsc --noEmit` (node v24.15.0, package-local): exit 0.
- Focused `node --test test/enrollment-input.test.ts`: **1/1 pass, natural
  624.229166 ms**. This is the only test asserting the import output; no other
test or module consumes that wording.

The combined DEFAULT concurrent installed-enabled full-suite result above
(156/157, natural 318.6 s, strict pass, unchanged source/native hashes) remains
attributed to candidate `ac32ae0d2` and is preserved as-is; the wording repair
is validated separately by this delta. Full-suite 149/149 evidence for `3dbf9aaaa`
(natural 357.645 s + strict) remains preserved in `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/`
and the earlier strict+focused CLI-delta runs in `CLI_INTEGRATION_5A40.md`; none
were rerun for this wording-only change.

Publication state recorded at the commit boundary below.

## Publication record (updated at commit boundary)

- Delta commit `41f45d59e` (restore pending offline import guidance + this file).
- Pushed `beehive/demo-transport-473233b8` to approved origin
  `https://github.com/block/buzz.git`: `6caa4150d..41f45d59e` (six commits:
  3dbf9aaaa, 1315af75c, ac781fd95, 7176028b7, ac32ae0d2, 41f45d59e).
- Verified remote head `41f45d59ec2693530b2dcb6bb55b137dc06823ed` == local HEAD;
  working tree clean. No other branch, PR, merge or release action taken.
- This publication-record edit is committed as a follow-up commit on the same
  branch so handoff evidence survives cancellation.

## FIRST_DEMO.md review against implemented CLI

Verified commands and guards exist as documented: `setup` (create/approve/import),
`provision-agent`, `reconcile-provision`, `auth-info`, `catalog`, private
`host <dir> <relay> --owner-present` (guard: private installation without the flag
exits with the OS-credential consent error), `tui <catalog> <relay>` with hidden
owner signer, TUI commands `hosts/select/show/save/start/restart/stop/quit`,
credential helper 10 s deadline, 10 s admission verification window, fresh
single-use membership proof with close/reopen requirement after transport loss.
Owner-run steps, prerequisites owner/operator must supply, and commands not
executed by workers are in `FIRST_DEMO.md`.

Result verified and recorded above; publication record follows.
