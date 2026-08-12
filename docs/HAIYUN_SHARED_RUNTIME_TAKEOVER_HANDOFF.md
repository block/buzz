# HAIYUN Shared Runtime Takeover Handoff

This document hands the Codex Desktop takeover fix to a separate coding agent.
It is an evaluation change for Buzz Codex Lab, not a production deployment.
Do not include Buzz identity keys, invite URLs, backup passwords, private task
content, or relay credentials in commits or test logs.

## Source State

- Repository: `https://github.com/chemyibinjiang/buzz.git`
- Branch: `codex/buzz-shared-appserver`
- Starting commit before this handoff: `a97692f0`
- Existing evaluation release: `codex-lab-v0.5.8-cefdea77`
- Shared runtime URL: `ws://127.0.0.1:51919`

The CHEMDA source worktree contains unrelated local changes that must not be
reverted, reformatted, staged, or included in this fix:

- `desktop/src/features/settings/ui/PrivateKeyBackupRow.tsx`
- `desktop/tests/e2e/profile-backup-settings.spec.ts`
- untracked `.tmp/` and generated `crates/buzz-acp/C...` test artifacts

## User Requirement

When Codex Desktop is still running with its private backend, Buzz must not
silently claim that Desktop was opened on the shared runtime. Buzz should:

1. Detect the conflicting Codex Desktop/private app-server processes.
2. Explain that Desktop has not fully exited and that active turns or unsaved
   composer drafts may be interrupted.
3. Require an explicit confirmation before takeover.
4. On confirmation, force-close only the Codex Desktop process tree and its
   package-local private backend.
5. Preserve the Buzz-owned shared app-server on port `51919`.
6. Relaunch Codex Desktop with `CODEX_APP_SERVER_WS_URL` pointing to `51919`.
7. Verify that Desktop did not create another private app-server.
8. Treat a thread writer conflict as a user-actionable blocked condition, not a
   generic crash that should be retried automatically.

## Reproduced Process Topology

HAIYUN showed these relevant processes:

```text
PID 27912
C:\Users\yibinjiang\AppData\Local\OpenAI\Codex\bin\...\codex.exe
  -c features.code_mode_host=true app-server
  --listen ws://127.0.0.1:51919

PID 2744
C:\Users\yibinjiang\AppData\Local\Buzz Codex Lab\buzz-acp.exe app-server

PID 7036
C:\Program Files\WindowsApps\OpenAI.Codex_...\app\resources\codex.exe
  -c features.code_mode_host=true app-server --analytics-default-enabled
```

Interpretation:

- PID 27912 is the correct long-lived Buzz shared app-server.
- `buzz-acp.exe app-server` is the expected stdio/WebSocket bridge.
- PID 7036 is a second, package-local Codex Desktop app-server using its default
  private transport. It is the conflicting writer.
- Each app-server also had its own `codex-code-mode-host.exe` child; those
  children are expected and confirm that two independent backends existed.

Do not kill processes by the filename `codex.exe`: that would also terminate
the correct shared runtime. Identify targets from the installed AppX package
manifest and verified executable paths.

## Failure Chain

The shared runtime repeatedly logged:

```text
thread-store conflict: thread ... already has an active writer
```

It then logged repeated failures connecting to the per-agent MCP endpoint:

```text
http://127.0.0.1:23120/mcp
```

Port `23120` is not a second central app-server. It is the identity-scoped MCP
server injected into the task-bound ACP session. The task resume fails first;
the attempted agent startup then tears down that MCP server. Automatic runtime
reconciliation retries the failed startup, repeatedly creating and destroying
the MCP endpoint and producing the transport-error tail. The writer conflict is
the root condition; the MCP errors are downstream noise.

Upstream behavior is intentional:

- A second app-server process must reject `thread/resume` while another process
  owns the writer.
- Re-resuming the running thread through the same app-server process succeeds.
- WebSocket app-server transport remains experimental/unsupported upstream.

Primary references:

- `https://github.com/openai/codex/blob/main/codex-rs/app-server/tests/suite/v2/thread_resume.rs`
- `https://github.com/openai/codex/blob/main/codex-rs/app-server/README.md`

## Existing Bug In Buzz

`desktop/src-tauri/src/managed_agents/codex_tasks.rs` currently launches Desktop
by setting `CODEX_APP_SERVER_WS_URL` only in the new child process. If Codex
Desktop is already running, Electron can activate/reuse the existing process,
so the new environment variable has no effect. The command still returns
success and the UI says `Opening Codex Desktop on the shared runtime` without
verifying the resulting process topology.

Avoid persisting `CODEX_APP_SERVER_WS_URL` globally in this patch unless a
complete disable/uninstall cleanup path is added. A stale user-level variable
would break normal Codex Desktop startup after Buzz Codex Lab is uninstalled.

## Suggested Backend Changes

### Runtime Status

Extend `CodexSharedRuntimeStatus` in:

- `desktop/src-tauri/src/managed_agents/codex_tasks.rs`

Suggested fields:

```rust
pub desktop_process_ids: Vec<u32>,
pub private_app_server_process_ids: Vec<u32>,
pub desktop_detection_error: Option<String>,
```

On Windows, collect a process snapshot using the exact AppX manifests for
`OpenAI.Codex` and `OpenAI.CodexBeta`:

1. Resolve each package's application executable from
   `Get-AppxPackageManifest`.
2. Identify running Desktop processes by exact normalized executable path.
3. Identify private package backends only when the executable is the package's
   `app\resources\codex.exe`, the command line contains `app-server`, and it is
   not the verified shared `--listen ws://127.0.0.1:51919` command.
4. Never classify the runtime under
   `%LOCALAPPDATA%\OpenAI\Codex\bin\...\codex.exe` listening on `51919` as a
   takeover target.

Use one blocking process-snapshot call per status refresh. Do not run slow WMI
queries on the async/UI thread.

### Confirmed Takeover Command

Add a separate Tauri command instead of adding an implicit force flag to the
normal launch action, for example:

```rust
take_over_codex_desktop_shared(confirmed: bool)
```

Require `confirmed == true`. The command should:

1. Snapshot exact Desktop/private-backend targets.
2. Determine Desktop process-tree roots and force-close those roots and their
   children. Also terminate verified orphan private backends if necessary.
3. Wait until all verified targets have exited, with a bounded timeout.
4. Re-probe `ws://127.0.0.1:51919`; abort if the shared runtime was lost.
5. Launch the AppX application executable with
   `CODEX_APP_SERVER_WS_URL=ws://127.0.0.1:51919` in its child environment.
6. Wait briefly, then re-snapshot processes.
7. If a new package-local private app-server appears, close the newly launched
   Desktop tree and return an actionable failure instead of leaving two writers.
8. Return the updated `CodexSharedRuntimeStatus` so the UI can update at once.

The ordinary `launch_codex_desktop_shared` command should refuse to launch when
a Desktop process already exists outside shared mode and direct the UI to the
confirmed takeover action.

Register the new command in:

- `desktop/src-tauri/src/commands/codex_tasks.rs`
- `desktop/src-tauri/src/lib.rs`

## Suggested Frontend Changes

Update the API surface in:

- `desktop/src/shared/api/codexTaskTypes.ts`
- `desktop/src/shared/api/codexTasks.ts`
- `desktop/src/features/agents/codexSharedRuntimeHooks.ts`

Update `CodexSharedRuntimePanel.tsx`:

- When `privateAppServerProcessIds` is non-empty, show an amber conflict state
  instead of the normal green ready state.
- State that Codex Desktop is running outside the shared runtime.
- Show a **Take over Codex Desktop** command.
- Use a controlled `AlertDialog` for confirmation.
- The confirmation must warn that active turns and unsaved drafts may stop.
- Name the exact shared endpoint that will remain running.
- Use **Cancel** and **Close and reconnect** actions.
- Disable duplicate actions while takeover is pending.
- Refetch/update shared-runtime status from the command response.

Update `CodexTaskAgentDialog.tsx` so `ready` plus a private-backend conflict is
not considered usable. Do not load the task list or enable **Create agent**
until the conflict is resolved.

## Writer-Conflict And Retry Handling

Update `friendlyAgentLastError.ts` to recognize both:

```text
already has an active writer
already has a live local writer
```

This must work even when the structured JSON-RPC code is `-32600`; the current
unknown-code branch returns raw text too early. Suggested copy:

```text
This Codex task is open in a separate Codex Desktop runtime. Open Codex shared
runtime settings, take over Desktop, then retry the agent.
```

Do not present a generic **Retry** action for this condition because it will
reproduce the conflict.

The startup reconciliation path currently retries failed relays at 5 seconds,
30 seconds, and 2 minutes. Relevant files:

- `desktop/src/features/agents/useManagedAgentRuntimeReconciliation.ts`
- `desktop/src/features/agents/managedAgentReconciliationPlan.ts`

Classify a runtime row whose `error` contains a writer-conflict marker as a
terminal/user-actionable failure. Keep the failed runtime row visible, but count
that relay as reconciled for automatic retry scheduling. Manual retry remains
available after takeover. This should collapse the `23120/mcp` cascade to one
failed attempt.

Also review `useManagedAgentFailureNotifications.ts`: suppress its **Retry**
toast action for writer conflicts or replace it with navigation to the shared
runtime resolution UI.

## Required Tests

### Rust

- Parse a zero, one, and multiple-process snapshot.
- Distinguish the `%LOCALAPPDATA%` shared runtime from a WindowsApps private
  backend.
- Refuse ordinary launch when Desktop is already running privately.
- Require explicit confirmation for takeover.
- Verify takeover scripts target exact package paths and never PID/name-match
  the `51919` runtime.
- Verify a post-launch private backend produces a failure.
- Existing `codex_tasks` tests must remain green.

### Frontend

- Runtime status type/API mapping includes process IDs.
- Private backend renders the conflict state and takeover action.
- Cancel performs no mutation.
- Confirm invokes takeover exactly once and refreshes status.
- Task creation remains disabled during conflict/takeover.
- Writer-conflict errors receive actionable copy even with code `-32600`.
- Reconciliation does not schedule automatic retries for writer conflicts.
- Other transient reconcile failures still use the existing capped backoff.
- Failure notifications do not offer a blind retry for writer conflicts.

## Manual Acceptance Test On HAIYUN

1. Start Codex Desktop normally so it creates a WindowsApps private app-server.
2. Open Buzz Codex Lab and confirm it shows the Desktop conflict.
3. Select takeover, then cancel. Confirm no process exits.
4. Select takeover again and confirm.
5. Verify the WindowsApps private app-server exits while the `%LOCALAPPDATA%`
   app-server continues listening on `51919`.
6. Verify Codex Desktop reopens and no second private app-server appears.
7. Start the bound task agent and send a DM. Confirm it replies.
8. Open the same task in Codex Desktop. Confirm there is no writer-conflict UI.
9. Repeat with an active Desktop turn and confirm the takeover warning is clear.
10. Reproduce an intentional private-writer conflict and confirm only one agent
    startup attempt occurs and the `23120/mcp` errors do not repeat.

Process inspection command:

```powershell
Get-CimInstance Win32_Process |
  Where-Object {
    $_.Name -match 'codex|chatgpt|buzz-acp' -or
    $_.CommandLine -match 'app-server'
  } |
  Select-Object ProcessId, ParentProcessId, Name, ExecutablePath, CommandLine |
  Format-List
```

## Scope Boundaries

- Do not change ALIYA firewall, relay, Docker Compose, SSH, DNS, or media state.
- Do not expose port `51919`; it must remain loopback-only.
- Do not kill processes by a broad name match.
- Do not add a public endpoint or authentication workaround.
- Do not change the Codex task binding identity model.
- Keep the patch focused on Windows shared-runtime takeover and terminal
  writer-conflict behavior.

## Continuation Prompt

```text
Read docs/HAIYUN_SHARED_RUNTIME_TAKEOVER_HANDOFF.md and implement the confirmed
Codex Desktop takeover flow on branch codex/buzz-shared-appserver. Preserve all
unrelated worktree changes. Detect exact AppX Desktop/private-backend processes,
never terminate the Buzz-owned ws://127.0.0.1:51919 runtime, add a confirmation
dialog and post-launch verification, and make active-writer failures terminal
for automatic reconciliation so the 23120 MCP retry cascade stops. Run focused
Rust and frontend tests, then report changed files, test results, commit hash,
and any remaining Windows limitations.
```
