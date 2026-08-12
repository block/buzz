# HAIYUN Codex Lab Test Handoff

This handoff covers the private Buzz + Codex shared-runtime evaluation on the
HAIYUN Windows computer. Do not add relay credentials, identity keys, backup
passwords, invite URLs, or private Codex task content to this repository.

## Source Snapshot

- Fork: `https://github.com/chemyibinjiang/buzz.git`
- Branch: `codex/buzz-shared-appserver`
- Baseline commit: `cefdea77244df6dd42e92d94715a4133c94c8113`
- Evaluation release: `codex-lab-v0.5.8-cefdea77`
- Product name: `Buzz Codex Lab`
- Shared app-server: `ws://127.0.0.1:51919`

Clone and verify the handoff source on HAIYUN:

```powershell
git clone --filter=blob:none --branch codex/buzz-shared-appserver https://github.com/chemyibinjiang/buzz.git
Set-Location buzz
git rev-parse HEAD
git status --short --branch
```

Use the published evaluation installer for the first test. A fresh source clone
does not contain the generated Windows sidecars, so building locally is a
separate developer workflow documented in `docs/CODEX_LAB_WINDOWS_TESTING.md`.

## Expected Topology

- Buzz Codex Lab and Codex Desktop run on HAIYUN.
- Both connect to the same HAIYUN-local Codex app-server on port `51919`.
- `127.0.0.1` in the Buzz UI means HAIYUN itself, not CHEMDA or ALIYA.
- Buzz Room history and uploaded media remain on the private ALIYA relay.
- Codex task history, execution state, credentials, and workspace files remain
  on HAIYUN unless the user explicitly uploads an artifact.

## First Test

1. Install and open Buzz Codex Lab.
2. Join the private evaluation community using an invite supplied out of band.
3. Fully quit Codex Desktop before enabling shared runtime for the first time.
4. In Buzz, select **Enable shared runtime** and wait for `ready`.
5. Select **Open Codex Desktop** so it uses the same shared app-server.
6. Select **Add Codex task**, find an existing HAIYUN task, select the result,
   name the Buzz agent, and create it.
7. The first task-list request can take several seconds while the local Codex
   history is loaded. If it remains on `Loading Codex tasks...` for more than
   30 seconds, collect diagnostics instead of repeatedly recreating the agent.

## Test Matrix

Record pass/fail and an approximate response time for each item:

1. DM the agent: `Reply exactly: HAIYUN_SHARED_RUNTIME_OK`.
2. Mention it in a channel and confirm one visible reply is published.
3. Open the same task in Codex Desktop and confirm prior messages are present.
4. Start a turn in Codex Desktop, then mention the agent in Buzz. Confirm Buzz
   reports the task as busy and that wait/steer behavior is understandable.
5. Stop the Buzz agent, complete one local Codex turn, restart the agent, and
   confirm the next Buzz message continues the same task.
6. Upload a PNG containing unique text and confirm the task reads the pixels,
   not only the attachment metadata or URL.
7. Upload a UTF-8 Markdown file containing Chinese text, a table, and a code
   block. Confirm download, agent reading, and rendered reply preserve UTF-8.
8. Send a PNG from the agent and confirm Buzz renders a preview and download.
9. Restart Buzz Codex Lab and repeat the DM without rebinding the task.
10. Restart the HAIYUN shared runtime through the supported UI and confirm Codex
    Desktop and the Buzz agent reconnect without losing task identity.

## Local Runtime Diagnostics

Run these commands in PowerShell on HAIYUN while the problem is visible:

```powershell
Invoke-WebRequest -UseBasicParsing http://127.0.0.1:51919/readyz
Invoke-WebRequest -UseBasicParsing http://127.0.0.1:51919/healthz
Get-NetTCPConnection -LocalPort 51919 -State Listen |
  Select-Object LocalAddress, LocalPort, OwningProcess
Get-NetTCPConnection -LocalPort 51919 -State Listen |
  ForEach-Object { Get-Process -Id $_.OwningProcess }
```

Return the following with any failure:

- Exact visible error and local timestamp.
- Whether `readyz` and `healthz` returned successfully.
- The task ID, but no private task transcript.
- Whether the failure occurred in DM, channel, image upload, steering, or start.
- Whether retrying reused the same agent identity or created a duplicate.
- A screenshot with secrets and private content removed.

## Known Observation

On the first HAIYUN run, the Add Codex task dialog connected successfully to
`ws://127.0.0.1:51919` but briefly displayed `Loading Codex tasks...`. The list
eventually loaded without restarting anything. Treat a short cold load as a
performance observation; treat a repeatable load longer than 30 seconds as a
UI/runtime bug requiring a timeout, retry control, and request tracing.

## Codex Continuation Prompt

After opening this clone in Codex on HAIYUN, use:

```text
Read docs/HAIYUN_TEST_HANDOFF.md and continue the HAIYUN Buzz Codex Lab
evaluation from the pinned source revision. Do not expose or commit invite
links, nsec keys, backup passwords, API credentials, or private task content.
Run the local runtime checks, execute the test matrix, record exact timings and
errors, and implement narrowly scoped fixes only for reproducible failures.
Preserve unrelated worktree changes.
```
