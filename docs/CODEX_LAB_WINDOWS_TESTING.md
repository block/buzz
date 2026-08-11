# Buzz Codex Lab: Windows Evaluation Build

This build is an isolated, unsigned evaluation package for testing Buzz with
Codex Desktop on Windows. It is not a production Scientist Room deployment.

## What The Installer Contains

- Buzz Desktop with Codex task binding and shared-runtime controls.
- The Buzz ACP harness, developer MCP, agent, CLI, and Nostr credential helper.
- No Buzz identity private key, API token, community URL, relay address, Codex
  credentials, or machine-specific path.
- No automatic updater. Test builds stay on the version that was installed.
- No Codex binary, Codex credentials, local task history, or workspace files.
  The shared runtime connects to the tester's existing Codex Desktop install.

The app uses the product name `Buzz Codex Lab`, the identifier
`xyz.chemyibinjiang.buzz.codexlab`, and the deep-link scheme
`buzz-codex-lab://`. It can coexist with the upstream Buzz application and
keeps a separate local application-data directory.

## Tester Prerequisites

1. Windows x64.
2. Codex Desktop installed, signed in, and opened normally at least once so its
   local runtime bundle is complete.
3. A private Buzz community URL or invite supplied by the evaluator.
4. Network access to that community.

The installer is unsigned. Windows may show an unknown-publisher warning; the
evaluator should send the SHA-256 checksum with the installer so testers can
verify the exact file before continuing.

## First Run

1. Install and open Buzz Codex Lab.
2. Create a new Buzz identity or import the tester's own existing identity.
   Never send an `nsec` private key to another person.
3. Join the evaluation community using the URL or invite supplied privately.
4. Fully quit Codex Desktop.
5. In Buzz, open Agents or Settings and select **Enable shared runtime**.
6. When the status is ready, select **Open Codex Desktop**. This launches Codex
   Desktop against the same loopback app-server used by Buzz.
7. Select **Add Codex task**, choose an existing local task, name its Buzz
   identity, and create the agent.

Only one model turn can run on a Codex task at a time. If Codex Desktop is
already running a turn, Buzz should show that the task is busy; wait for it to
finish or intentionally steer the active turn.

## Smoke Test

1. Send the task agent a DM: `Reply exactly: SHARED_RUNTIME_OK`.
2. Confirm the reply appears in Buzz.
3. Open the same task in Codex Desktop and confirm its prior history remains.
4. Stop the Buzz agent, work locally for one turn, restart the Buzz agent, and
   confirm the next DM continues the same task.
5. Send a PNG and confirm the agent can inspect it and reply.
6. Restart Buzz Codex Lab and confirm the community, messages, and task binding
   reconnect without recreating the identity.

## Uninstall And Reset

Uninstall **Buzz Codex Lab** from Windows Settings like a normal application.
The uninstaller removes the isolated program directory and leaves evaluation
state under `%APPDATA%\xyz.chemyibinjiang.buzz.codexlab` so a reinstall can
reconnect. To perform a completely clean reset, first export any identity that
must be kept, uninstall the app, and then remove only that Codex Lab data
directory. Do not remove the upstream Buzz data directory.

## Diagnostics To Return

- Windows version and Codex Desktop version.
- Buzz Codex Lab `BUILD-INFO.json` commit and installer SHA-256.
- Whether shared runtime reached `ready`.
- Whether the DM, local continuation, image, and restart checks passed.
- Exact visible error text and timestamp for any failure. Do not send identity
  private keys, API tokens, or full private task transcripts.

## Build The Installer

From a Windows x64 PowerShell session at the repository root:

```powershell
.\scripts\build-codex-lab-windows.ps1
```

Artifacts are written to `dist/codex-lab-windows/`:

- `Buzz-Codex-Lab_<version>_<commit>_x64-setup.exe`
- `SHA256SUMS.txt`
- `BUILD-INFO.json`

The build script clears relay, identity-injection, reconnect-hook, updater, and
signing environment variables before compiling. It rebuilds and verifies every
bundled Windows sidecar before creating the NSIS installer.

Rust and native source paths are remapped or trimmed during release builds. A
stable incremental Cargo cache is kept under
`%LOCALAPPDATA%\BuzzCodexLabBuild`; pass `-BuildCacheDirectory` to override it.
