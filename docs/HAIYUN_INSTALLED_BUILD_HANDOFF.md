# HAIYUN Installed Build Handoff

This handoff records the Windows package built and installed on HAIYUN after
the shared Codex app-server and figure-delivery fixes. It is intended for the
next agent that updates or rebuilds Buzz Codex Lab.

Do not commit relay credentials, Buzz identity keys, invite URLs, backup
passwords, private Codex task content, or generated installer binaries.

## Published Source

- Repository: `https://github.com/chemyibinjiang/buzz.git`
- Branch: `codex/buzz-shared-appserver`
- Fixed source commit: `58b2a9d9f81d65092488674b385fea57bb3c05d2`
- Immediate previous commit: `e5fed28b` (`docs: hand off shared runtime takeover fix`)
- Older evaluation package baseline: `cefdea77`
- Product: `Buzz Codex Lab 0.5.8`
- Shared app-server: `ws://127.0.0.1:51919`

The implementation comparison requested for this handoff is:

```powershell
git diff --stat e5fed28b..58b2a9d9
git diff --name-status e5fed28b..58b2a9d9
```

That range changes 29 files with 1,960 insertions and 502 deletions. The
generated installer is ignored by `/dist/` and is not part of the Git commit.

## What Changed From The Previous Commit

Commit `e5fed28b` documented the required takeover behavior but did not contain
the implementation. Commit `58b2a9d9` implements it:

1. Detects exact Windows AppX Codex Desktop and package-local private
   `codex.exe app-server` processes without broadly killing by process name.
2. Preserves the Buzz-owned `%LOCALAPPDATA%` shared runtime listening on port
   `51919`.
3. Refuses the ordinary Desktop launch while a conflicting private backend is
   active.
4. Adds an explicit **Take over Codex Desktop** confirmation flow with
   **Cancel** and **Close and reconnect** actions.
5. Rechecks the process topology after relaunch and fails if Desktop creates a
   new private backend.
6. Treats active-writer errors, including JSON-RPC code `-32600`, as
   user-actionable terminal failures instead of automatically retrying and
   cascading into per-agent MCP transport errors.
7. Preprocesses agent-generated PNG/JPEG figures before upload, retains the
   scientific figure pixel budget and aspect ratio, and removes metadata.
8. Uploads local figure paths through the authenticated Buzz media path and
   returns relay-hosted resources to the conversation.
9. Allows native media download from the configured relay's exact HTTP origin
   while retaining private-address, redirect, MIME, and size protections.
10. Recognizes ACP image resource links in the Desktop transcript.

Key implementation areas:

- `desktop/src-tauri/src/managed_agents/codex_desktop.rs`
- `desktop/src/features/agents/ui/CodexSharedRuntimePanel.tsx`
- `desktop/src/features/agents/managedAgentReconciliationPlan.ts`
- `desktop/src/features/agents/lib/friendlyAgentLastError.ts`
- `crates/buzz-cli/src/image_upload.rs`
- `desktop/src-tauri/src/commands/media_download.rs`
- `desktop/src/features/agents/ui/agentSessionImageContent.ts`

## Built Installer

The verified local artifact is:

```text
dist/codex-lab-windows/Buzz-Codex-Lab_0.5.8_58b2a9d9f81d_x64-setup.exe
```

- Size: `53,811,975` bytes (about 51.3 MiB)
- SHA-256: `16ac80d622dd85f85339b5fbde4f14cf6535f0b353b01052692d64cceb6ae99b`
- Signature: unsigned evaluation build
- Target: `x86_64-pc-windows-msvc`
- Built at: `2026-08-13T08:28:42Z`

The source worktree was clean when the package workflow generated
`BUILD-INFO.json`. The installed sidecars matched the hashes in that file. The
installer completed successfully and installed under:

```text
%LOCALAPPDATA%\Buzz Codex Lab
```

The package is intentionally not committed. Copy it out of band or produce a
new installer from the published source. Do not infer the source revision only
from the displayed application version, which remains `0.5.8`; use the commit
embedded in `BUILD-INFO.json` and the installer filename.

## Build Environment Notes

The successful package build required:

- Visual Studio 2022 Build Tools with the native C++ workload and recommended
  Windows SDK components.
- The repository Hermit toolchain.
- A short Cargo target directory to avoid Windows MSBuild FileTracker
  `FTK1011` path-length failures.
- A cached, checksum-verified sherpa-onnx archive after a transient GitHub TLS
  download failure.

The successful workflow was:

```powershell
scripts\build-codex-lab-windows.ps1 -SkipDependencyInstall
```

The short build cache used on HAIYUN is under
`%LOCALAPPDATA%\BuzzCodexLabBuild`. It is disposable build output, not source.
At handoff time it occupied about 13.26 GiB. Do not delete it without the
owner's approval; retaining it makes the next rebuild substantially faster.

## Validation Completed

The source at `58b2a9d9` passed 101 focused checks:

- 54 frontend shared-runtime, retry, status, takeover, and image-content tests.
- 3 Rust figure preprocessing/upload tests.
- 5 Rust Windows takeover/process-classification tests.
- 31 Rust native media/download security tests.
- 8 official package-workflow runtime-control checks.
- Desktop TypeScript typechecking also passed.

Live HAIYUN acceptance established:

- Buzz showed the private-runtime conflict and disabled task creation.
- **Take over Codex Desktop** opened the destructive-action warning.
- **Cancel** closed the warning without terminating either runtime.
- The shared app-server remained healthy at `/readyz` and `/healthz` with
  `200 OK`.
- Buzz and Codex Desktop subsequently used the same task history. A Buzz DM
  produced a Codex turn whose final answer was automatically signed and
  published back to Buzz by `buzz-acp`.

The final destructive **Close and reconnect** path was not executed during the
original Codex debugging turn because it would have terminated that active
turn. Re-test it only after saving or finishing active Desktop work.

## Relay And Proxy Finding

The relay at `59.77.33.211:4500` is healthy when reached directly over
Tailscale:

- NIP-11 metadata: `200 OK`
- WebSocket upgrade: `101 Switching Protocols`
- Direct source address on HAIYUN: `100.71.241.45`

The observed `502 Bad Gateway` was returned by the local HTTP proxy at
`127.0.0.1:7890`, not by the remote Buzz relay. HAIYUN had proxy variables for
HTTP, HTTPS, WS, WSS, and SOCKS, while `NO_PROXY` initially omitted the relay
IP. The current user-level bypass is:

```text
NO_PROXY=localhost,127.0.0.1,::1,wsl.localhost,59.77.33.211
```

After restarting only Buzz, it established a direct TCP connection from the
Tailscale interface to `59.77.33.211:4500`, messages loaded, and the red `502`
banner disappeared. Before diagnosing ALIYA, compare a normal request with:

```powershell
curl.exe -sv --noproxy '*' http://59.77.33.211:4500/ `
  -H 'Accept: application/nostr+json'
curl.exe -sv http://59.77.33.211:4500/ `
  -H 'Accept: application/nostr+json'
```

Do not change ALIYA firewall, relay containers, Docker configuration, DNS, or
media state merely because the proxied request returns `502`.

## Remaining Acceptance Work

1. Finish or save any active Codex Desktop turn.
2. In Buzz, open the shared runtime conflict panel.
3. Select **Take over Codex Desktop**, then **Close and reconnect**.
4. Confirm the WindowsApps private app-server exits while the
   `%LOCALAPPDATA%` listener on `51919` remains healthy.
5. Confirm Desktop reopens without spawning another private app-server.
6. Send a DM to the task-bound agent and verify exactly one threaded reply.
7. Ask the agent to generate one new PNG figure.
8. Verify Buzz uploads it to relay media, renders it, and downloads it over the
   configured exact HTTP relay origin.

Old broken figure links cannot be repaired automatically when the original
local files were never uploaded. Validate with a newly generated figure.

## Instructions For The Next Agent

```text
Read docs/HAIYUN_INSTALLED_BUILD_HANDOFF.md and compare
e5fed28b..58b2a9d9 before changing the package. Continue on
codex/buzz-shared-appserver, preserve unrelated worktree changes, and do not
commit generated files under dist/. Verify the current HEAD before attributing
test results. Complete the remaining takeover and new-figure acceptance tests.
If source changes are required, run the relevant full package checks, rebuild
the Windows installer, verify BUILD-INFO.json and SHA256SUMS.txt, install it,
and report the new commit, filename, size, and SHA-256. Keep
59.77.33.211 in NO_PROXY so local proxy failures are not mistaken for relay
failures.
```
