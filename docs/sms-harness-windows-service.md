# Running the SMS operator harness durably (Windows)

The SMS bridge only works while `buzz-acp` is running. This document covers
hosting it so it survives a closed terminal, a logoff, and a reboot.

## Why this runs on a workstation and not in a container

The harness dispatches agents against **local repo checkouts** via
`--project-paths` (e.g. `bidcraft=E:/Projects/buildbid/bidcraft-repo`), and its
agent command is a locally installed, locally authenticated Claude Code CLI. A
container has neither. Cloning the repo inside one would also change the
semantics from "work on my working copy" to "work on a clone", which is not
what the SMS operator flow is for.

So the durable host is a **Windows Scheduled Task** on the machine that owns
those checkouts. `nssm` is deliberately not a dependency.

## The failure this is designed around

The harness was once started by hand and later exited with code 1 and *nothing
fatal in its log* — no panic, no error, just absence. From outside, a dead
harness is indistinguishable from a healthy one that nobody happens to be
texting. Inbound messages are accepted by the relay and simply never answered.

Two consequences shape the design:

- **Restarting is not enough; liveness must be observable.** The supervisor
  republishes `heartbeat.txt` every 60s with a UTC timestamp and both PIDs.
  A stale heartbeat is the signal that something is wrong, and it does not
  depend on the harness flushing its stdout.
- **A crash loop must be legible, not loud.** Restarts back off exponentially
  (5s → 300s) when the harness dies within 60s of starting, so a
  misconfiguration produces a readable log rather than gigabytes of identical
  lines.

## Components

| File | Role |
|---|---|
| `scripts/sms-harness-supervisor.ps1` | Runs the harness, restarts it, holds the key, writes the heartbeat. The task launches **this**, never `buzz-acp.exe`. |
| `scripts/sms-harness-task.ps1` | Registers/removes the scheduled task; `-ProvisionSecret`, `-Status`, `-Verify`. |
| `~/.buzz-sms-harness/` | Install root — key blob, logs, pid, heartbeat. **Outside the repo, always.** |

## Setup

```powershell
# 1. Store the Nostr key as a DPAPI blob (decryptable only by this user,
#    on this machine). Prompts; never takes the key as an argument.
pwsh -File scripts/sms-harness-task.ps1 -ProvisionSecret

# 2. Register the scheduled task.
pwsh -File scripts/sms-harness-task.ps1 -Install

# 3. Start it now rather than waiting for the next logon.
pwsh -File scripts/sms-harness-task.ps1 -Start

# 4. Confirm it is actually alive.
pwsh -File scripts/sms-harness-task.ps1 -Status
```

`-Status` prints the task state, heartbeat age, and redacted log tails.
`-Verify` asserts the install's invariants and fails loudly if any drifted.

To stop it without uninstalling, `-Stop` drops a `stop.flag` the supervisor
checks before every relaunch, which avoids racing a restart.

## Flags that look redundant and are not

Three arguments are load-bearing. Each produces a harness that connects,
reports healthy, and does nothing if removed:

- **`--kinds 9`** — `--subscribe all` with no kinds subscribes to *nothing*;
  kinds default to an empty vec.
- **`--respond-to anyone`** — the default is `owner-only`, which silently
  drops every event when no owner is configured.
- **`CLAUDECODE` unset** — `claude-code-acp` refuses to start when it is
  inherited, failing as a `-32603` that reads like an ACP protocol fault
  rather than an environment guard.

## The environment scrub

`buzz-acp`'s clap parser is built with the `env` feature, and **48 of its flags
have environment twins** (`crates/buzz-acp/src/config.rs`). Three of them —
`BUZZ_ACP_SUBSCRIBE`, `BUZZ_ACP_KINDS`, `BUZZ_ACP_RESPOND_TO` — silently
override exactly the arguments above. An inherited `BUZZ_ACP_RESPOND_TO=owner-only`
yields a harness that subscribes, logs nothing unusual, and drops every message.

The supervisor therefore clears the entire `BUZZ_ACP_*` namespace (plus
`CLAUDECODE`, `BUZZ_AUTH_TAG`, `BUZZ_API_TOKEN`, `BUZZ_PRIVATE_KEY`) before
reading any configuration, and logs the variable *names* it removed. This is
what makes the committed argument list mean what it says.

## Key handling

`BUZZ_PRIVATE_KEY` is a live Nostr private key.

- Stored as a **DPAPI blob**, decryptable only by the provisioning user on the
  provisioning machine. Copying the install root to another machine yields a
  supervisor that refuses to start rather than one that runs unauthenticated.
- Passed to the child through the **process environment**, never a command
  line — `Win32_Process.CommandLine` is readable by any user on the box.
- The install root is ACL-restricted to the current user, and `-Verify`
  re-checks that rather than assuming it held.
- `-Install` refuses an `-InstallRoot` inside a git working tree, and
  `.gitignore` carries matching patterns as a second layer.
- Log tails printed by `-Status` are masked for 64-hex and `nsec1` strings.
  The harness child holds the key in its environment, so a panic that dumps
  env or a debug-level log line could otherwise put it on screen — and
  `-Status > file` would copy it somewhere unprotected.

## Troubleshooting

**Texts get no reply.** Check `-Status` first. A heartbeat older than ~2
minutes means the supervisor is not running; missing entirely means it never
started. Note that relay logs showing nothing for `sms_sink` is the *correct*
signal for "the agent posted no reply" — it is evidence about the harness, not
the relay.

**Task registered but never runs.** Confirm the trigger fires in a session
that has the user's credentials and can reach the `E:` drive. A task running
as SYSTEM before logon has neither.

**Two replies per message.** More than one harness is subscribed. The
supervisor holds a `Local\BuzzSmsHarnessSupervisor` mutex and reaps a recorded
orphan PID on startup, but a harness started by hand from a terminal is outside
that guard.
