# Buzz Desktop Manual Testing

Manual runbooks for desktop behavior that automated tests cannot observe. A
developer, or an agent that can operate a desktop session, follows each step
and records what it observed. For the automated suites, see
[TESTING.md](../TESTING.md) and `pnpm test:e2e:smoke`.

---

## Windows native notifications

Unit tests cover permission mapping, the delivery sequencing, and the click
activation queue. They cannot observe real toasts, the per-app switch in
Windows Settings, or state that must survive a restart. Run this runbook for
changes to `desktop/src-tauri/src/commands/notifications.rs` or the Windows
paths in `desktop/src/features/notifications/`.

### 1. Prerequisites

- Windows 11 with an interactive desktop session. Toasts do not appear in a
  service session or over a headless SSH connection.
- Native Windows Node.js, pnpm, and Rust (MSVC) on `PATH`. The files under
  `bin/` are Hermit shell wrappers and do not run from PowerShell.
- One Buzz account in a community where you can create a private channel. The
  echo workflow in step 4 plays the other party, so you need neither a second
  account nor a local relay.

### 2. Build and install

Releases ship only the per-user NSIS installer, so test that rather than a
bare `buzz-desktop.exe` or the MSI. From the repository root in PowerShell:

```powershell
cargo build --release -p buzz-acp -p buzz-agent -p buzz-dev-mcp `
	-p git-credential-nostr -p buzz-cli

$target = "x86_64-pc-windows-msvc"
$binaries = "desktop\src-tauri\binaries"
New-Item -ItemType Directory -Force $binaries | Out-Null
"buzz-acp", "buzz-agent", "buzz-dev-mcp", "git-credential-nostr", "buzz" |
	ForEach-Object {
		Copy-Item -Force "target\release\$_.exe" "$binaries\$_-$target.exe"
	}

Get-Process buzz-desktop -ErrorAction SilentlyContinue | Stop-Process
pnpm -C desktop tauri build --bundles nsis
& (Get-ChildItem desktop\src-tauri\target\release\bundle\nsis\*-setup.exe).FullName
```

The installer puts Buzz in `%LOCALAPPDATA%\Buzz` and creates
`Start Menu\Programs\Buzz.lnk` carrying the `xyz.block.buzz.app`
AppUserModelID.

### 3. Observe IPC (optional)

To see which permission state the frontend reads and whether it sends a
toast, start Buzz with the WebView2 debugging port and open `edge://inspect`
in Edge:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9222"
& "$env:LOCALAPPDATA\Buzz\buzz-desktop.exe"
```

Look for `windows_notification_permission_state` (expect `"granted"` or
`"denied"`) and `show_native_notification`.

### 4. Set up the echo workflow

A workflow reply is signed by the relay and tags the workflow owner, so Buzz
treats it as someone else mentioning you and raises a toast.

1. Turn on **Settings > Experiments > Workflows**.
2. Create a private channel named `notification-echo`.
3. Open **Manage channel**, go to **Workflows**, choose **New workflow**,
   select the **YAML** tab, and paste:

   ```yaml
   name: Notification echo
   trigger:
     on: message_posted
     filter: 'str_contains(trigger_text, "echo-test") && !trigger_is_reply'
   steps:
     - id: wait
       action: delay
       duration: 30s
     - id: channel_echo
       action: send_message
       text: 'Echo: {{trigger.text}}'
     - id: thread_echo
       action: send_message
       reply_in_thread: true
       text: 'Thread echo: {{trigger.text}}'
   ```

To trigger toasts, post `echo-test 1` (increment the number each time) in
`notification-echo`, then switch to another channel or app within 30 seconds.
You should get two toasts: **Echo:** as a channel message and **Thread echo:**
as a reply in that message's thread. **Remind me later** set two minutes out
also works as a trigger. A DM toast needs a second account, because the
workflow `send_dm` action is not implemented.

### 5. Checks

1. **Launch with an existing shortcut:** start Buzz twice from the Start Menu.
   Both launches open a window. A regression exits with code 101 before any
   window appears.
2. **First toast on a new account:** sign in to a Windows account where Buzz
   has never shown a toast, such as a new local user. Turning on desktop
   notifications in Buzz settings succeeds without an error banner, the first
   trigger shows a toast, and Buzz then appears under **Settings > System >
   Notifications**.
3. **Restart:** quit Buzz, confirm no `buzz-desktop.exe` remains with
   `Get-Process buzz-desktop`, and start it again without touching the toggle.
   The toggle is still on and the next trigger shows a toast.
4. **Off in Windows Settings:** turn Buzz off under **Settings > System >
   Notifications**, then switch back to Buzz. Its desktop notification toggle
   turns off. Turning it on shows "Desktop notifications are blocked for
   Buzz", and triggers show no toast. Turn Buzz back on in Windows, turn the
   Buzz toggle on again, and the next trigger shows a toast. Buzz does not turn
   its own toggle back on automatically.
5. **Click while running:** click the **Echo:** toast. Buzz comes to the front
   and focuses that message in the channel timeline without opening an empty
   thread panel. Click the **Thread echo:** toast; Buzz opens that thread.

Record the Windows build (`winver`), the Buzz version, and the result of each
check in the PR.

### 6. Clean up

Delete the echo workflow and the `notification-echo` channel. Uninstall Buzz
from **Settings > Apps > Installed apps** if the test machine should not keep
it. Remove the extra Windows account if you created one.

### 7. Run it with an AI agent

The agent needs to run PowerShell and to see and operate the Windows desktop.
Windows computer use works only in the foreground of an unlocked, interactive
session, and the agent takes over the mouse and keyboard while it works, so
prefer a VM or a separate Windows account.

Harnesses that can do both on Windows:

- **ChatGPT desktop app for Windows:** in Work or Codex, install **Plugins >
  Computer Use**; see [Computer Use](https://learn.chatgpt.com/docs/computer-use).
  It cannot operate terminal apps, so it runs the build through its own shell
  tool.
- **An MCP-capable agent plus [Windows-MCP](https://github.com/CursorTouch/Windows-MCP):**
  install [uv](https://docs.astral.sh/uv/) and Python 3.13 or newer, then
  register the server with your agent, for example:

  ```powershell
  codex mcp add windows-mcp -- uvx windows-mcp serve --exclude-tools Registry
  copilot mcp add windows-mcp -- uvx windows-mcp serve --exclude-tools Registry
  ```

  The first line is for Codex CLI and the second for GitHub Copilot CLI. The
  Windows-MCP README also covers Claude Desktop, Claude Code, and Gemini CLI.
  Windows-MCP has full system access and sends anonymous telemetry unless
  `ANONYMIZED_TELEMETRY=false` is set in its environment.

Browser-only agents, such as ChatGPT's cloud browser, cannot see a local
desktop app. The Claude API computer use tool can, but only with an agent loop
and desktop driver you provide yourself.

The agent cannot approve UAC prompts, sign in for you, or create Windows
accounts, so do those steps yourself: installing prerequisites, signing in to
Buzz, and check 2. Then paste a prompt like this into the agent:

```text
Follow the "Windows native notifications" runbook in desktop/TESTING.md of
this checkout. Build and install Buzz (step 2), set up the echo workflow
(step 4), and run checks 1, 3, 4, and 5. Do not read, type, or export private
keys or passwords; if Buzz asks to sign in, stop and ask me. Do not approve
UAC prompts. For each check, report pass or fail, what you observed, and a
screenshot of the toast or of the notification history. Include winver output
and the Buzz version. Clean up as in step 6 when done.
```
