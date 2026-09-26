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
- A second Buzz account that can send you a DM, or a message you can set a
  reminder on.

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

### 4. Trigger a toast

Set **Remind me later** on a message for two minutes out, or have the second
account send you a DM while Buzz is in the background. Confirm each toast on
screen and in the Windows notification history.

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
5. **Click while running:** click a toast for a channel message or DM. Buzz
   comes to the front and focuses that message in the channel timeline without
   opening an empty thread panel. Click a toast for a thread reply; Buzz opens
   that thread.

Record the Windows build (`winver`), the Buzz version, and the result of each
check in the PR.

### 6. Clean up

Uninstall Buzz from **Settings > Apps > Installed apps** if the test machine
should not keep it. Remove the second Windows account if you created one.
