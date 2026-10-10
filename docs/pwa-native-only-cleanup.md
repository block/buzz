# Web build — native-only feature cleanup list

Source: the web capability map (`desktop/src/platform/web/capabilities.ts`, 408 distinct commands; regenerate with `node scripts/pwa/capability-report.mjs`, `LIST=1` for per-command lists). Current policy is a labelled placeholder ("Available in the desktop app") unless noted. Linked from [pwa-plan.md](./pwa-plan.md) §11. Nothing here changes app code; each line is a recommendation for JM to approve. "Bridge A/B/C" = slices in the plan (A relay bridge, B Tailscale direct, C hosted tunnel).

Legend: REMOVE = delete the entry point in the web build; HIDE = don't render; PLACEHOLDER = labelled "Available in the desktop app"; BRIDGE = later served by slice A/B/C (placeholder until then).

| # | Feature (group, ≈cmds) | Why it can never work in a browser | Recommendation |
|---|---|---|---|
| 1 | Managed agents: create/start/stop/restart/delete, logs, runtime lifecycle (agents, 96 incl. rows 2–5) | Spawns local ACP/agent sidecar processes; browsers can't run local processes | BRIDGE A (list/status/start/stop/message/approve); placeholder until then |
| 2 | ACP runtime install, provider/auth discovery, custom harnesses, backend probes | Needs local binaries, filesystem and local CLIs | HIDE (setup is a desktop task); status only via A |
| 3 | Personas and teams runtime (create/edit/publish, snapshot import/export, agent cards, Bestie assignment) | Tied to local agent runtime and files; publishing metadata alone is possible but useless without a runtime | PLACEHOLDER; read-only listing could come via A |
| 4 | OpenClaw MCP / workspace tool calls, mesh-llm, local model status | Local MCP servers, local models, loopback ports | HIDE; BRIDGE A for status only |
| 5 | Observer/archived agent transcripts, agent metrics/usage, observed-unread | Data stored in local archive and runtime | BRIDGE A (transcripts, usage snapshots) |
| 6 | Pinned sites, Stage, Drive, playground, HTML preview, browser agent (webviews, 27) | Native WKWebView surfaces; iframes blocked by X-Frame-Options/CSP, no cross-origin control, no screen capture of other sites | PLACEHOLDER; HTML preview could be a sandboxed iframe later; Stage/browser-agent live view not feasible via A; B/C possible only for dev-server preview, not arbitrary sites |
| 7 | Huddles: join/leave, agents in huddle, voice settings (huddle_audio, 28) | Native LiveKit + cpal audio device layer | PLACEHOLDER now; candidate to rebuild on WebRTC/Web Audio as a separate project |
| 8 | Audio output device list/select, pocket voice, local TTS/voice registry | Local TTS/voice models and device enumeration via cpal; browsers limit output device selection | HIDE (browser TTS via speechSynthesis is a separate optional feature) |
| 9 | Projects: local repos, clone, checkout, worktrees, diffs, file content, open folder (git_projects, 25) | Needs local git, filesystem and the repos | BRIDGE A (read-only snapshots, basic diff/file view); BRIDGE B/C for large files; PLACEHOLDER until then |
| 10 | GitHub PR list/merge/remote branch via `gh` | Uses the local `gh` CLI and credentials | BRIDGE A (desktop runs `gh`); or later GitHub API with its own token; placeholder until then |
| 11 | Terminals / PTY and merge-recovery terminals (terminals, 13) | No shell access from a browser | BRIDGE B/C (live xterm over WebSocket); A too laggy; PLACEHOLDER until then |
| 12 | OS keyring, nsec access, import/persist identity, NIP-49 ncryptsec backups (keys_at_rest, 31) | Keys live in the signer (NIP-07/NIP-46) by design; page must never hold the nsec | HIDE (already policy "hide") |
| 13 | Recovery pairing / SAS pairing, identity archive and archive sync | Device-to-device native pairing and local archive storage | HIDE (web identity is the signer; archive sync stays desktop) |
| 14 | BuilderLab binding and community naming | Native flows tied to local identity binding and keyring | HIDE |
| 15 | System tray: agent activity indicator, tray actions | No tray/menu bar in a browser | REMOVE (use A notifications + PWA badge instead) |
| 16 | Haptics, sidebar default haptic | No haptics API on iOS Safari; Vibration API absent on iOS | REMOVE |
| 17 | OS idle seconds, prevent-sleep | Not exposed to pages (Wake Lock covers only screen, optional later) | REMOVE (optional Screen Wake Lock for huddles later) |
| 18 | Window vibrancy, title-bar double-click, traffic-light chrome | Native window chrome | REMOVE (plain web layout) |
| 19 | Pop-out windows list/open/focus | Native window management | PLACEHOLDER; replace with `window.open`/PWA window if wanted |
| 20 | Deep links `hulabuzz://`, pending deep-link handling | Custom URL scheme registered by the desktop app | REMOVE; use normal URLs/`web+` protocol handler later |
| 21 | Auto-updater, version/relaunch, baked build env | Web updates via service worker, not an installer | REMOVE (replaced by PWA update prompt) |
| 22 | Native notifications | Tauri native notification API | Replace with Notification API/Web Push (S5 server needed); BRIDGE A for agent events; until then HIDE |
| 23 | Copy image to clipboard (native) | Native clipboard image write | Replace with Async Clipboard API (works on most browsers); keep placeholder where unsupported |
| 24 | Media proxy port, admin origin override, relay reconnect hook | Loopback ports/native hooks | REMOVE |
| 25 | Media upload, pickers, downloads, link previews (proxy, 15) | Native pickers/fs; link-preview fetch blocked by CORS | NOT native-only: serve via `<input type=file>`, `/media/upload` and a BFF (S3); keep placeholder until built |
| 26 | Relay admin, moderation, membership admin (proxy, 21) | Not native-only: relay HTTP API with NIP-98 | Port in S6; placeholder until then |
| 27 | Remaining chat data (threads, DMs, forum, notes, canvas, search…; 56) | Not native-only; just not yet ported | Port in S2; not part of the cleanup |

Notes
- Rows 25–27 are listed for completeness; they will work in the browser and are not removal candidates.
- Row counts overlap loosely with the capability groups in `capabilities.ts` (agents 96, webviews 27, huddle_audio 28, git_projects 25, terminals 13, os_integration 10, keys_at_rest 31). The per-command lists are authoritative.
- Suggested default if JM wants a quick decision: REMOVE rows 15–18, 20, 21, 24; HIDE rows 2, 4, 8, 12–14; keep everything else as labelled placeholders that bridge slices later fill in.
