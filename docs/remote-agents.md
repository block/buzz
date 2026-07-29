# Remote agents over SSH

A **remote agent** is a managed agent whose harness runs on another host. The desktop still owns
the agent: it mints the agent's nostr key, holds the record, and renders it beside local agents.
It does not own the process. On the host, `buzz-acp` runs as a `systemd --user` unit, and the only
liveness signal the desktop has is the agent's presence on the relay — there is no status op, no
polling channel, and no open connection between deploys.

`buzz-backend-ssh` is the provider binary that puts it there. It is not bundled with the desktop:
`discover_provider_candidates` prepends the app bundle's own directory to the provider search path,
so shipping it inside the bundle would give every install an auto-discovered SSH-deploy capability
and quietly undermine the "only use providers from trusted sources" warning the create dialog
shows. Install it to `~/.local/bin`, which is already on the discovery path.

## Provider protocol

The desktop enumerates PATH (plus the executable's own directory and `~/.local/bin`) for files
named `buzz-backend-<id>`, and resolves `<id>` against `^[a-z0-9][a-z0-9_-]*$`. A trailing
executable extension (`.exe`, `.com`, `.cmd`, `.bat`) is not part of the id — Cargo installs the
provider as `buzz-backend-ssh.exe` on Windows, and reading that filename literally derives `ssh.exe`,
which the id rule rejects for the dot. Discovery deduplicates on the id, so a host carrying both
spellings offers one provider. It spawns the binary, writes one JSON request to stdin, closes it,
and reads one JSON response from stdout. One process per op; no daemon, no state, no version
negotiation.

`buzz-backend-ssh` implements five ops.

| op | opens SSH | provider budget | desktop budget | desktop caller |
|---|---|---|---|---|
| `info` | no | — | 10s | `probe_backend_provider`: the host field, and Settings → Remote servers to name and version each row |
| `check` | yes | 8s | — | none yet |
| `discover_harnesses` | yes | 40s | 60s | `WhereToRunSection`, on "check host" |
| `probe_models` | yes | 110s | 150s | `WhereToRunSection`, after a harness resolves |
| `deploy` | yes | 300s | 600s | `deploy_to_provider`, from create and from start |

The provider budget always fires first, so a timeout arrives as a structured error rather than as a
killed child. Any other `op` value is rejected before a connection is opened, so a typo costs a
parse and not an SSH handshake.

Every desktop-side entry point resolves the provider through `resolve_discovered_provider` before
spawning it, so a frontend or IPC caller that names a `binaryPath` cannot steer execution at an
arbitrary binary.

`info` is the only op that runs before a host is configured — it is what produces the host field —
so it never opens a session and never requires `provider_config`.

```json
{"op": "info", "request_id": "<uuid-v4>"}
```

```json
{"ok": true, "name": "SSH", "version": "…",
 "description": "Run agents on a remote host over SSH, supervised by systemd --user.",
 "config_schema": {"type": "object", "required": ["ssh_host"], "properties": {…}}}
```

`check` is a preflight: `echo buzz-ok` over the configured session. Failures are classified into
actionable guidance (`Permission denied` → `authorized_keys` / `tailscale set --ssh`,
`Host key verification failed` → known_hosts, `Could not resolve hostname` → address or tailnet,
`Connection refused`/`timed out` → reachability). Anything unclassified passes through verbatim
rather than being flattened.

`discover_harnesses` probes `buzz-acp` and every candidate harness in **one** generated `sh`
script. N sequential `ssh` invocations would spend the whole budget on handshakes over a
200 ms link and the harness picker would visibly hang. Two details in that script are load-bearing:
every probed child gets `</dev/null`, because the script itself arrives on the remote shell's stdin
and a child that reads stdin would swallow the rest of it; and each probe runs under `timeout 5`
where the host has it, so a harness whose `--version` opens a REPL cannot hold the budget.

```json
{"op": "discover_harnesses", "request_id": "…",
 "provider_config": {"ssh_host": "vps.example", "ssh_user": "ubuntu"}}
```

```json
{"ok": true,
 "buzz_acp": {"path": "/usr/local/bin/buzz-acp", "version": "0.4.26"},
 "harnesses": [
   {"id": "goose", "label": "Goose", "command": "goose", "args": ["acp"],
    "env": {"GOOSE_MODE": "auto"}, "installInstructionsUrl": "", "installHint": "",
    "available": true, "binaryPath": "/home/ubuntu/.local/bin/goose", "version": "goose 1.9.0"}]}
```

Every element is a `HarnessDefinition` in the desktop's own camelCase wire shape plus
`available`/`binaryPath`/`version`. Unresolved candidates are still reported with
`available: false`, so the picker can say "install this on the host" instead of hiding the option —
but `selectedRemoteHarness` filters the pin on `available`, so an entry a re-check turned
unavailable stops being the pin rather than deploying a command the host says is not installed.
`buzz_acp: null` with `ok: true` is likewise deliberate.

The `command` reported is the one that actually resolved on the host, not the first candidate:
`claude` resolves through `claude-agent-acp` or `claude-code-acp`, and the pin must name the binary
that exists there. `env` carries the runtime's `default_env`, which local spawn applies from the
catalog and a remote deploy can only get from here; the create flow pins it into the record's
`env_vars`, where it lands in the env file underneath any user-set value.

A host with the **Hermes** CLI advertises one extra entry per Hermes profile — `hermes-matt`,
"Hermes (matt)", `command: "hermes"`, `args: ["--profile", "matt", "acp"]` — because a profile is a
whole isolated `HERMES_HOME` (its own SOUL, memory, skills, credentials), so ten profiles are ten
different agents, and the plain `hermes-acp` entry can only ever run whichever one is *sticky*. That
entry stays as the default option, and `default` gets its own pin so `hermes profile use` cannot
strand it. The profiles are read from the directory store (`<root>/profiles/*/`, `HERMES_HOME`
honored and trimmed back to the root) in the same single script round trip, gated on `hermes`
resolving — `hermes profile list` is a human table with no `--json`, and the directory layout is
what Hermes itself resolves a profile against. Names are untrusted remote input on their way into an
id and an argv, so only `[a-z0-9][a-z0-9_-]*` (Hermes's own rule, and a subset of the desktop's
harness-id rule) is accepted; anything else is skipped whole rather than sanitized, and the count is
capped at 32 with the remainder logged to stderr. Absent Hermes, the catalog is byte-identical to
what it was before; present with a Hermes root but no `profiles/` store, it is the plain entry plus
`hermes-default`, since the root directory *is* the default profile; present with no Hermes root at
all, it is just the plain entry.

Those per-profile entries carry `"exclusive": true`, the catalog's one statement about identity: the
entry names a persistent identity on the host — its own memory, sessions and credentials — rather
than an ephemeral runner. Deploying `claude` or the plain `hermes-acp` entry N times to one host is
the point; pinning two agents to *the same profile* is two puppeteers driving one body, so the
desktop refuses the second. Every other entry omits the key, and an absent key means "deploy as many
as you like" — the flag is the only hermes-aware thing here, and the desktop reads it generically
(`isExclusiveRemoteHarnessAdded`): an entry counts as already taken when an existing agent is backed
by the *same provider and provider config* and pinned to the *same command and args*, in which case
the harness picker renders it disabled with an "(added)" suffix and auto-pick skips it. Config
equality is exact (after trimming, dropping blanks and sorting keys), not host resolution, so
`10.0.0.4` and `vps.tail1234.ts.net` read as different hosts and the guard simply does not fire —
it under-matches rather than ever falsely blocking a create. Resolving aliases needs a
host-identity answer from the provider, which is the real fix rather than a normalization table in
the desktop.

`probe_models` exports the harness env inside the script, then runs `buzz-acp models --json` on the
host and returns the document verbatim under `models_raw`. The desktop feeds it straight into the
same `normalize_agent_models` the local path uses, so the model picker needs no remote-specific
code. That host-side command carries its own budget inside the provider's 110s: `MODELS_TIMEOUT`,
60s, matched to what the normal agent-init path gives the same adapter spawn — a shorter one only
fails probes the real spawn would have survived, since a cold node adapter on a busy host takes
tens of seconds to reach `initialize`. Model env must be nested under `agent.env_vars` — that is the only place the desktop's
`env_secrets_from_request` scrubber looks. A flat `model_env` is accepted but loses that second
redaction layer.

`deploy` provisions and starts the unit, and returns `{"ok": true, "agent_id": "buzz-acp@<slug>"}`.
The desktop persists `agent_id` in `record.backend_agent_id`.

`deploy` also **verifies or installs** two host-side tools. When the payload carries the optional
path — a path on the *desktop* machine to a Linux binary — and the host resolves none, that binary is
installed to `~/.local/bin` inside the provisioning round trip, before anything else is written.
The fields are seams, not modes: there is no second op, no provisioning step, and no new UI state.
A payload carrying neither field sends no binary and opens no extra round trip — the script is the
one the crate has always sent, plus the CLI resolution block, which is unconditional because its
whole job is to notice a host that has no CLI. A test pins that script byte for byte.

| payload field | tool | installed as | host has neither it nor a payload |
|---|---|---|---|
| `agent.buzz_acp_binary` | `buzz-acp`, the harness | `~/.local/bin/buzz-acp` | **exit 90** — the deploy stops |
| `agent.buzz_cli_binary` | `buzz`, the agent-facing CLI | `~/.local/bin/buzz` | a `WARNING:` line on stderr; the deploy **continues** |

**The asymmetry is deliberate.** `buzz-acp` *is* the agent, so its absence is fail-closed. The `buzz`
CLI is what a remote agent's own system prompt tells it to reply with (`buzz messages send --reply-to
<event-id>`, `buzz feed get`) — a local agent gets it because the desktop bundles it as a sidecar and
prepends its directory to the spawned harness's `PATH`. Without it a remote agent still runs; it just
cannot use the CLI, and in practice spends its first minutes hunting the filesystem for a command
that is not there. That is worth a warning and never worth failing a deploy over. Integrity failures
(exit 93/94) are fatal for **both**: a payload that arrives damaged is evidence the stream is damaged,
and that stream also carries the minted nsec.

**Resolution is `PATH` *or* `~/.local/bin/<tool>`, never `PATH` alone.** A non-interactive SSH
command reads no profile, so `~/.local/bin` — the documented convention and the install destination —
is not on the ambient `PATH`. That is exactly why the unit's env file pins
`PATH="$HOME/.local/bin:$PATH"` itself (see the env file contract). A `command -v`-only rule would
therefore never see the copy a previous deploy installed: since deploy is the start path, every agent
start would re-stream tens of megabytes and swap the binary underneath a running fleet. The probe and
the deploy script apply the same two-part rule, so they cannot disagree.

**Staleness rule: push-when-missing only.** A host that already resolves a tool keeps the binary it
has, whatever its version. Deploy is the start path, so a version-comparing rule would reinstall
underneath a running fleet on every start, and a desktop pinned to an older artifact would
*downgrade* the host. Refreshing an existing install belongs to the release-artifact follow-up below.

**Setting either field costs one extra round trip, and only when one is set.** Deploy is the start
path, so embedding binaries unconditionally would stream tens of megabytes of base64 on every start of
every agent, forever, to hosts that were provisioned on day one. So when — and only when — at least
one field is present, `deploy` asks the host the resolution question above first, for both tools in a
single probe; anything the host already has is never even read from disk. The probe is an
optimization, never the decision: the deploy script re-checks on the host and installs only into an
empty variable, so a host that gains or loses a tool between the two round trips still lands correct.

The install rides the SSH stdin channel with everything else, which dictates its shape:

- **base64, not raw bytes.** The script is text; a NUL or a stray newline inside an ELF section
  would corrupt the *script*, not just the payload. The encoded alphabet (`A-Za-z0-9+/=`) contains
  no shell metacharacter and no `_`, so no data line can terminate a `BUZZ_ACP_B64_EOF` /
  `BUZZ_CLI_B64_EOF` heredoc early. The delimiters are quoted as well, so the remote shell expands
  nothing in either body, and they differ so one script can carry both.
- **sha256 before install.** The digest is computed on the desktop and travels in the script in the
  clear (a fingerprint, not a credential); the host runs `sha256sum -c` against the decoded temp
  file and aborts with exit 94 on a mismatch. Nothing is made executable before it verifies.
- **Atomic.** Decode goes to `~/.local/bin/.<tool>.tmp.$$` — same directory as the target, so the
  `mv` is a rename — then `chmod 755`, then `mv`. Every failure path removes the temp file first, so
  no run leaves a half-written executable where `ExecStart` would name it.
- **`base64` and `sha256sum` must exist on the host** (coreutils). Their absence is exit 92 with a
  clear message, never a silent skip of the integrity check.
- **The desktop refuses the payload before the session opens** when the path is missing, is not a
  file, is empty, is over 200 MB, or is not an ELF binary — and the message names which tool it is
  about. Pushing a Mach-O from a macOS desktop would otherwise install cleanly and restart-loop on
  `Exec format error` every five seconds after a deploy that reported success. This holds for the CLI
  too: a *missing* CLI is tolerable, but a desktop that pointed the seam at the wrong file has a bug
  worth naming.
- **The secret discipline is untouched.** The pushed bytes are not secret, but they share the stream
  with the minted nsec; base64 is what keeps them from corrupting it. `umask 077`, the `chmod 600`
  env file and the "nothing secret on any argv" rule are unchanged.
- **Only `buzz-acp` reaches the unit.** `ExecStart` is substituted from the resolved harness path.
  The CLI is reached purely through the env file's `PATH`, which is why installing it and pinning
  that `PATH` are one change and not two.

The fields are filled desktop-side from the `BUZZ_ACP_PUSH_BINARY` and `BUZZ_CLI_PUSH_BINARY`
environment variables, read at deploy time (`deploy_payload_json`), so a developer can point either at
a fresh build without restarting the app. Those are dev/dogfood seams, not the destination: the
release build should resolve the artifacts for the host's platform by version, with no user-visible
path at all. **Fetching release artifacts is out of scope here and is the immediate follow-up**, along
with the version-refresh rule that only becomes safe once the desktop knows which version it is
offering.

Non-fatal host-side complaints — today, only the missing-CLI warning — reach the desktop on the
provider's **stderr**, prefixed `WARNING: ` and scrubbed by the same redactor the failure path uses.
`invoke_provider` writes them to the desktop log (`tracing::warn`) when the op succeeds, and folds
them into the error message when it fails. The op's JSON response is unchanged either way: the deploy
succeeded, and a warning is not a result.

Errors are `{"ok": false, "error": "…"}` on stdout, human detail on stderr, and **exit 0 always**.
A non-zero exit makes `invoke_provider` discard stdout entirely and report raw stderr, which throws
the structured error away.

A failure the user can act on may carry an optional `recovery` alongside `error`:

```json
{ "ok": false,
  "error": "this host requires Tailscale SSH authentication in a browser: https://login.tailscale.com/a/…",
  "recovery": { "action": "open_url", "url": "https://login.tailscale.com/a/…" } }
```

`recovery` is optional in both directions, so there is no negotiation and no flag: a desktop that
does not read it still renders `error`, which names the problem and carries the URL as text, and a
desktop that does read it finds nothing there from an older provider. The only `action` today is
`open_url`, and the only URL is Tailscale's login host — the SSH provider **constructs** that URL
from a fixed prefix plus a charset-constrained token rather than parsing one out of remote output,
so no host, scheme, or query from the host can reach the browser opener. The desktop re-validates
the prefix anyway before opening, on the same "the provider is a subprocess, not a trusted peer"
footing as its secret re-redaction.

The provider emits this when a tailnet ACL uses Tailscale SSH's `check` action, which makes `ssh`
print the URL and then block for a human that `BatchMode` cannot supply. It is detected by peeking
at buffered stderr during the poll loop, so the op fails in one 25 ms tick instead of burning its
whole budget (8 s for `check`, 300 s for `deploy`) and reporting a bare timeout.

On the desktop, `invoke_provider` returns `ProviderFailure { message, recovery }` rather than a
`String`, and `ProviderRecovery::from_response` is where the URL is re-validated — on entry, so an
unvalidated one never exists in desktop memory at all and no later reader of the payload can become
a second, unguarded way to open it. There is deliberately no `From<ProviderFailure> for String`:
that is the type-level guard against a caller flattening the recovery away, which is the one bug
this plumbing exists to prevent. The provider commands carry the type out to the frontend, which
reads it off `TauriInvokeError.payload` via `providerRecoveryOf`.

Two paths drop the recovery **explicitly**, each at one named site, because their surface cannot
render an action: `start_managed_agent` (a toast) and `create_managed_agent`'s `spawn_error` (a
reported field of a succeeding create). Nothing is lost to the user there — the message names the
problem and carries the URL as text — but widening either needs its surface to grow an action
first. The agent record's `last_error` is a plain string for a different reason: it is read back
long after the fact, and an auth URL is a one-shot token that is stale by then.

**Recovery is a manual retry.** The create dialog renders an "Authenticate in browser" button beside
the failure and nothing else: the desktop cannot tell when the user has finished authenticating in a
browser it does not own, so an auto-retry would be guessing at a delay. "Check the host again" is
already the retry, and it is the same button every other host failure offers.

## Configuration

`validate_provider_config` rejects any config key whose word-split contains
`secret`/`password`/`token`/`key`/`credential`, and drops it silently. That is why the identity
field is `ssh_identity_file` and not `ssh_key_path`.

| key | required | notes |
|---|---|---|
| `ssh_host` | yes | hostname, IP, or `user@host`. Rejected if it starts with `-` or contains whitespace/control characters. Carries a `oneOf` of tailnet devices when one is available. |
| `ssh_user` | no | Ignored when `ssh_host` already contains `@`. |
| `ssh_port` | no | Number or numeric string, `1..=65535`. Default 22. |
| `ssh_identity_file` | no | Passed as `ssh -i`. Defaults to `~/.ssh/config` and the agent. |
| `buzz_acp_path` | no | Absolute path to `buzz-acp` on the host. Defaults to whatever is on the host's PATH. |

There is no `unit_scope`. All deploys are `systemctl --user`.

The `oneOf` is a **generic decoration, not an SSH feature**. Any provider may attach
`oneOf: [{ const, title }]` to any config property; the desktop renders a dropdown over the
`const` values labelled by `title`, always with an "Other…" row that swaps back to the plain
text field. Nothing in the desktop knows what a tailnet is, and a value the list does not
contain — one carried over from before the decoration existed, or a peer that has since left
the tailnet — stays in the text field rather than reading as unselected. Omit the `oneOf` and
the field is exactly the text input it was before.

## Host prerequisites

`scripts/provision-buzz-host.sh` checks all of these on a candidate host and prints what is
missing. It is a preflight, not an installer.

1. **A non-root user.** The whole flow is root-free. The env file lands under that user's
   ownership, beside the harness credentials that already live there (`~/.claude`,
   `~/.config/goose`).

2. **`loginctl enable-linger <user>`.** This is the one non-obvious prerequisite. Without lingering,
   the user manager is torn down when the last session ends, so the agent is killed the moment the
   deploy's own SSH session closes — which reads as a flaky agent, not as a configuration problem.
   Lingering also creates `/run/user/$(id -u)`, without which every `systemctl --user` call fails
   to reach the bus. `deploy` runs `loginctl enable-linger` itself, before any bus traffic, but
   best-effort: some hosts gate it behind polkit, and failing it must not fail an otherwise good
   deploy. On those hosts, run it once by hand as root.

3. **`buzz-acp` on the host's PATH or at `~/.local/bin/buzz-acp`** — `deploy` resolves both, since a
   non-interactive SSH `PATH` does not contain the latter — or an absolute path in
   `buzz_acp_path`. `discover_harnesses` reports its absence without failing. `deploy` installs it
   when the desktop supplied one (`BUZZ_ACP_PUSH_BINARY`, see the `deploy` section) and otherwise
   refuses. Installing it needs `base64` and `sha256sum` on the host — coreutils, present on any
   normal Linux — and nothing else.

   **The `buzz` CLI is the same story with a softer ending.** Agents are told by their system prompt
   to reply with `buzz messages send`, so a host without it produces an agent that cannot. `deploy`
   resolves it the same two ways, installs it from `BUZZ_CLI_PUSH_BINARY` when the host has none, and
   otherwise emits a warning and provisions the agent anyway. Not a prerequisite — but a host that
   satisfies it gets noticeably better agents.

4. **At least one harness CLI**, named exactly as `discover_harnesses` probes it. Most harnesses
   require only their ACP adapter: `codex-acp` for Codex, `goose` for Goose, `cursor-agent`, `omp`,
   `grok`, `opencode`, `kimi`, `amp-acp`, `hermes-acp`, `openclaw`, or `buzz-agent`. Claude is the
   deliberate exception: it requires both `claude-agent-acp` or `claude-code-acp` **and** the
   vendor `claude` CLI whose stable launcher is bound into the adapter.

5. **SSH key auth.** Every invocation is `BatchMode=yes`, so a password prompt is an immediate
   failure and never a hang. Add the desktop machine's public key to `~/.ssh/authorized_keys`, or
   run `tailscale set --ssh` on the host.

6. **Tailscale (optional).** When the desktop's own `tailscale status --json` reports
   `BackendState: "Running"`, its peers decorate the `ssh_host` field as a device picker. Phones and
   TVs are filtered out; `Self` is never offered. The label carries reachability and, when the peer
   advertises `sshHostKeys`, a `· Tailscale SSH` marker — that field's absence is the negative
   signal, not an unknown. Tailscale absent, logged out, or empty produces a schema byte-identical
   to the plain one; manual SSH is the unchanged fallback.

`XDG_RUNTIME_DIR` needs no host action: a non-interactive SSH command often gets none, and `deploy`
sets it when the session did not supply one.

Windows hosts are never deploy targets. The provider runs on Windows — it resolves
`%SystemRoot%\System32\OpenSSH\ssh.exe` before PATH and suppresses the console window for every
child — but the remote side is POSIX `sh` and `systemd --user` throughout.

## Security invariants

These are properties of the code, not conventions to uphold.

- **Secrets cross on stdin only.** Every op sends its script to a remote `sh -s`; the remote argv is
  the literal string `sh -s`, and the local argv is ssh options. The remote `ps` is world-readable
  and the desktop's redaction has no reach there, so a secret on the remote argv would leak the
  agent identity to every user on the box.
- **The env file is owner-only.** Written under `umask 077`, `chmod 600`, then moved into place, so
  a failed write never leaves a half-written identity behind.
- **A pushed `buzz-acp` cannot corrupt the script carrying the nsec.** It travels base64-encoded
  inside a quoted heredoc, so no byte of it is ever read as shell syntax. It is verified against a
  desktop-computed sha256 before it is made executable, and installed by a same-directory rename, so
  a damaged or interrupted push leaves nothing runnable behind.
- **A deploy without the minted nsec fails closed.** An agent that mints its own key on the host
  looks deployed and is permanently unreachable: presence, mentions, `!shutdown`, badges and the
  NIP-OA auth tag all key off the pubkey the desktop minted.
- **A deploy without the harness pin fails closed.** The pin is the only channel by which the
  harness choice reaches the host. A blank one would fall through to `buzz-agent`, silently
  provisioning a harness the user never chose, so it is refused rather than substituted.
- **Reserved env keys are refused**, as are env names that are not POSIX identifiers and env values
  containing control characters. A newline in a value would otherwise end the assignment and start a
  line of the value's own choosing — including one that re-sets `BUZZ_PRIVATE_KEY`. The list is a
  verbatim copy of the desktop's `RESERVED_ENV_KEYS`, so a leak needs two independent failures.
- **`Secret` renders as `[REDACTED]`** in both `Debug` and `Display` and zeroizes on drop.
  `Agent` and `ssh::Output` deliberately do not derive `Debug` at all: the first holds provider API
  keys in plain `String`s, the second holds raw remote stderr, and only `Output::failure()` runs
  that through the scrubber.
- **Host-key trust is never relaxed for a typed address.** `StrictHostKeyChecking=ask` by default;
  `accept-new` only for an address this machine's own Tailscale daemon lists as a peer, which was
  already reached over a WireGuard-authenticated tunnel.
- **Provider binaries are resolved by discovery, never by name.** Every deploy, start and probe path
  resolves through `discover_provider_candidates`, so a frontend or IPC caller that names a
  `binaryPath` cannot steer execution at an arbitrary binary and feed it the agent's private key.

## The systemd unit

One templated `buzz-acp@.service` per host, instantiated per agent.

```ini
[Unit]
Description=Buzz agent %i
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=0

[Service]
Type=simple
EnvironmentFile=%h/.config/buzz-acp/%i.env
ExecStart=@BUZZ_ACP_BIN@
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
```

- `StartLimitIntervalSec=0` — a long-running agent must never be rate-limited into staying down. A
  unit held by the start limiter looks exactly like an agent that silently died, and only
  `systemctl reset-failed` clears it.
- `EnvironmentFile` — holds the minted nsec; systemd reads it as the owning user.
- `ExecStart` is an absolute path, substituted at install time from the host's resolved `buzz-acp`.
  systemd does not expand environment variables in the program position, and the shell indirection
  that would work around that is not worth adding to a unit whose environment carries a private key.
  The substitution is shell parameter expansion, not in-place editing: `sed -i` is a GNU extension
  that BSD and macOS hosts reject. Resolution runs *first*; the install only fills an empty `$acp`,
  so a deploy that installed `buzz-acp` writes the path of the copy it just installed, not a stale
  one. The path is written **double-quoted, per systemd's command-line syntax** — `ExecStart=` splits
  an unquoted value on whitespace, and `buzz_acp_path` may legitimately name a directory containing
  some, which would otherwise make systemd run the first word with the rest as arguments. `\` and `"`
  are escaped on the way in, since systemd unquotes C-style escapes inside double quotes.

The instance name is the agent name made unit-safe — lowercased, non-alphanumerics collapsed to `-`,
truncated to 32 characters — followed by the first 12 hex characters of the agent's `pubkey`. The
name is the readable half; **the pubkey fragment is the identity**. A display name is not unique:
two agents called "Research Bot" on one SSH account keyed on the name alone shared one unit, one env
file and one `agent_id`, so the second deploy overwrote the first agent's minted nsec and starting
either record drove whichever identity was written last. `deploy` therefore refuses a payload whose
`agent.pubkey` is absent or is not a 64-character hex key, rather than falling back to the name.

## Env file contract

The local spawn contract from `runtime.rs`, transcribed. Values resolved on the host — the absolute
harness path, `git-credential-nostr`, `PATH` — are appended by the remote script.

| var | value |
|---|---|
| `BUZZ_ACP_AGENT_COMMAND` | the pinned harness, resolved on the host with `command -v` |
| `CLAUDE_CODE_EXECUTABLE` | for a Claude ACP adapter, `~/.local/bin/claude` when executable, otherwise the host's `claude` launcher resolved from `PATH` |
| `PATH` | `$HOME/.local/bin:$PATH`, **expanded by the host's shell at deploy time** — see below |
| `BUZZ_PRIVATE_KEY` | payload `private_key_nsec` |
| `BUZZ_RELAY_URL`, `BUZZ_AUTH_TAG` | payload (auth tag omitted when absent) |
| `BUZZ_ACP_AGENT_ARGS` | comma-joined |
| `BUZZ_ACP_MCP_COMMAND` | empty |
| `BUZZ_ACP_LAZY_POOL` | `true` — the pool warms on the first accepted event instead of at startup; queued work is not dropped, and a restarted or idle unit does not hold N harness subprocesses |
| `BUZZ_ACP_AGENTS` | payload `parallelism` |
| `BUZZ_ACP_MULTIPLE_EVENT_HANDLING` | `steer` |
| `BUZZ_ACP_DEDUP` | `queue` |
| `BUZZ_ACP_RELAY_OBSERVER` | `true` |
| `BUZZ_ACP_RESPOND_TO` (+ `_ALLOWLIST`) | payload; `allowlist` mode with an empty list is refused |
| `BUZZ_ACP_SYSTEM_PROMPT`, `BUZZ_ACP_MODEL` | payload, omitted when empty |
| runtime model/provider env | payload `model` / `provider`, under the runtime's own names — see below |
| `BUZZ_ACP_IDLE_TIMEOUT`, `BUZZ_ACP_MAX_TURN_DURATION` | emitted only when set, so the harness's own defaults win |
| `NOSTR_PRIVATE_KEY`, `GIT_TERMINAL_PROMPT`, `GIT_CONFIG_*` | only when `git-credential-nostr` is on the host |
| user `env_vars` | written last, so they override — matching the local layering |

**The `PATH` line is the remote half of the desktop's own PATH contract.** Local spawn prepends
`<home>/.local/bin` (and the bundled sidecar directory) to the spawned harness's `PATH`
(`managed_agents::runtime::path::build_augmented_path`), which is why a local agent can run the
`buzz` CLI its system prompt tells it to reply with. Remotely the harness runs under `systemd --user`,
whose `PATH` is the user manager's: no profile, no login shell, and on many distributions no
`~/.local/bin` at all. Without this line every tool `deploy` installs would be installed and
unreachable.

It is composed **by the host's shell during the deploy**, not written into the unit as
`Environment=PATH=$HOME/.local/bin:$PATH`. systemd expands no variable in `Environment=` or in an
`EnvironmentFile`, so that form would hand the harness the five literal characters `$PATH`. The right
half is the non-interactive SSH shell's own `PATH`, captured at deploy time — which is the same
`PATH` the install machinery just searched, so anything `command -v` found on the host stays findable
for the agent. The harness passes its environment to its children unchanged, so this is what makes
`buzz` a command a remote agent can actually run.

The runtime model/provider pair is the remote half of `runtime_metadata_env_vars`. `BUZZ_ACP_MODEL`
is what `buzz-acp` reads; these are what the harness underneath it reads, and local spawn writes
both. Without them a remote Goose would fall back to whatever `~/.config/goose/config.yaml` on the
host says, with the user's model pick silently ignored.

| pinned command | model var | provider var |
|---|---|---|
| `goose` | `GOOSE_MODEL` | `GOOSE_PROVIDER` |
| `buzz-agent` | `BUZZ_AGENT_MODEL` | `BUZZ_AGENT_PROVIDER` |

The lookup is keyed on the command's file name, so an absolute pin
(`/home/ubuntu/.local/bin/goose`) resolves to the same runtime — matching `known_acp_runtime`
locally. Runtimes absent from the table declare no such vars in `KNOWN_ACP_RUNTIMES` either:
Claude is `provider_locked`, and neither Claude nor Codex has a model env var. An unset payload
field writes no key at all.

Claude has one additional executable binding. When the pinned harness is
`claude-agent-acp` or `claude-code-acp`, deploy prefers the stable
`~/.local/bin/claude` launcher and otherwise resolves `claude` from the host's
`PATH`, then writes it as `CLAUDE_CODE_EXECUTABLE`. This matches local desktop
spawn behavior and prevents the adapter from silently using the point-in-time
Claude binary bundled with its SDK dependency. The launcher path is preserved
instead of dereferencing its native-install symlink, so newly spawned ACP
children follow subsequent Claude Code updates. Deploy fails with exit 95
before writing the unit when the adapter is present but the Claude CLI is not.

`BUZZ_MANAGED_AGENT` is deliberately absent. It is the desktop's process-ownership marker for
reclaiming orphaned local children; where systemd owns the lifecycle it would be actively
misleading.

`turn_timeout_seconds` is deliberately never read. The payload still carries it, but
`BUZZ_ACP_TURN_TIMEOUT` is deprecated and ignored by the harness, and local spawn does not write it
either — `idle_timeout_seconds` and `max_turn_duration_seconds` are the live controls. A test pins
that no `TURN_TIMEOUT` key can reappear in the env file.

## Lifecycle

**Deploy is the start path.** `start_managed_agent` re-enters `deploy_to_provider`, so start and
redeploy are one code path and everything in it is idempotent. Non-idempotence would surface as
duplicate units, not as an error. One deploy is one round trip (plus the cheap resolution probe,
only when a push field is set) that resolves — or, on a host that has none and a payload that
carries one, installs — `buzz-acp` and the `buzz` CLI, resolves the
harness, writes the env file atomically, enables lingering, installs the unit template,
`daemon-reload`s only when the unit content actually changed, then `enable --now` and `restart`.
The restart is what makes an already-running unit adopt the rewritten env file.

**Stop is `!shutdown`.** The desktop's `stop_managed_agent` command rejects non-local agents
outright. The frontend sends a signed `!shutdown` @mention; the harness consumes it, drains
in-flight prompts, publishes `offline` presence, and exits. `Restart=always` then restarts the unit
after `RestartSec=5` — a `!shutdown` stops the current process, not the unit. To stop the unit,
`systemctl --user stop buzz-acp@<slug>.service` on the host.

**There is no `undeploy` op.** Deleting a deployed remote agent requires `force_remote_delete: true`
and permanently orphans a systemd unit and an env file containing an nsec on the host. This is the
strongest candidate for the immediate follow-up PR.

**Logs** are `journalctl --user -u buzz-acp@<slug> -f` on the host.

## Troubleshooting

**"Failed to connect to bus" during deploy.** Lingering is off and `loginctl enable-linger` was
rejected (polkit), so `/run/user/$UID` does not exist and no `systemctl --user` call can reach the
user manager. Run `sudo loginctl enable-linger <user>` once and redeploy.

**Agent goes online, then offline as soon as the deploy finishes.** Same cause, softer symptom: the
bus was reachable through the deploy's own session, and the user manager was torn down with it.
Enable lingering.

**`buzz-acp not found on the server's PATH or in ~/.local/bin` (exit 90).** Neither `command -v
buzz-acp` nor `~/.local/bin/buzz-acp` resolved on the host, and the payload carried no binary to
install. Install it to `~/.local/bin`, set `buzz_acp_path`, or
point `BUZZ_ACP_PUSH_BINARY` at a Linux `buzz-acp` on the desktop and let the deploy install it.
Note that `discover_harnesses` reports this non-fatally, so it can first appear at deploy time.

**`WARNING: no 'buzz' CLI on the server's PATH or in ~/.local/bin`.** The deploy **succeeded** — this
is a warning on the provider's stderr, not an error. The agent is running, but it cannot answer with
`buzz messages send --reply-to …` the way its own system prompt tells it to, so it will fall back to
slower replies (and, left to itself, waste its first turns looking for the command). Install `buzz`
into `~/.local/bin` on the host, or point `BUZZ_CLI_PUSH_BINARY` at a Linux `buzz` on the desktop and
redeploy. This is the one host-side complaint that is deliberately not fatal.

**Agent replies are slow, or it reports it cannot find `buzz`.** Either the CLI is not installed
(above), or it is installed somewhere the unit's `PATH` does not reach. The env file pins
`PATH="$HOME/.local/bin:<the host's PATH at deploy time>"`; `systemctl --user show-environment` and
`cat ~/.config/buzz-acp/<slug>.env` show what the harness actually got. A tool installed after the
last deploy into a directory that was not on the deploying shell's `PATH` needs one redeploy.

**`the server has no 'base64' / 'sha256sum'` (exit 92).** The host is missing coreutils, so a pushed
binary cannot be decoded or — the part that is not negotiable — verified. Install coreutils, or
install the tool on the host by hand. The deploy stops before writing anything.

**`the pushed buzz-acp` / `the pushed buzz did not decode` / `failed its sha256 check` (exit 93 /
94).** The binary was damaged between the desktop and the host. Nothing is installed and no temp file
survives; the env file and unit are never written. Fatal for both tools — including the
non-load-bearing CLI, because the damaged stream is the one carrying the nsec. Retry, and if it
repeats, the local file named by the corresponding `BUZZ_*_PUSH_BINARY` is the suspect.

**`the buzz-acp binary to push is not a Linux (ELF) executable`** (or `the buzz binary …`).
`BUZZ_ACP_PUSH_BINARY` / `BUZZ_CLI_PUSH_BINARY` points at the desktop's own build (Mach-O or PE)
rather than a Linux one. Caught locally, before the session opens; the message names which variable to
fix. The alternative is a unit that restart-loops on `Exec format error`, or a `buzz` on the host that
fails on every invocation, after a deploy that reported success.

**The model list never arrives, or reports `agent timed out (60s)`.** There are three nested
budgets on that path and the innermost one is on the host: `buzz-acp models --json` allows
`MODELS_TIMEOUT` — 60s, deliberately the same budget the normal agent-init path gives the very same
adapter spawn — for the harness to reach `initialize`. Outside it, the provider allows 110s for the
whole `probe_models` op and the desktop allows 150s. A cold node adapter (`codex-acp`) on a busy
host can take tens of seconds on its first spawn and be instant warm, so a probe that times out
once and succeeds on "check host again" is a slow host, not a broken harness. A probe that keeps
hitting 60s is the harness failing to start: run `buzz-acp models --json` on the host by hand, where
the adapter's own stderr is visible. Longer than 110s and the failure changes shape — the provider
budget fires first and the desktop reports a provider timeout rather than an agent one.

**`harness <name> not found on the server's PATH` (exit 91).** The pinned harness is not installed
under that name. Deploy stops before writing anything — no env file, no unit. Install the ACP
adapter and re-run discovery so the pin names a binary that exists.

**`Claude Code CLI not found in ~/.local/bin or on the server's PATH` (exit 95).** A Claude ACP
adapter is installed, but the vendor CLI it drives is not. Install Claude Code through its native
installer so `~/.local/bin/claude` exists, or put another `claude` launcher on the deploying
shell's `PATH`, then redeploy. The adapter's bundled SDK binary is deliberately not used.

**`Permission denied (publickey)`.** `BatchMode=yes` means SSH declined rather than prompting. Add
the public key to `~/.ssh/authorized_keys`, or `tailscale set --ssh` on the host.

**`Host key verification failed`.** The host key is not in `known_hosts`, and `BatchMode` cannot
prompt to accept it. Connect once with `ssh` by hand to review and accept the key. This is expected
for any manually typed address; tailnet peers are exempt.

**The device dropdown disappeared.** `tailscale status --json` no longer reports
`BackendState: "Running"` — most often a logged-out daemon, which exits 0 with
`BackendState: "NeedsLogin"`. The field degrades to plain text and manual SSH still works; a
MagicDNS name typed into it will fail with `Could not resolve hostname`.

## Known limitations

- `runtimeSupportsLlmProviderSelection` is a hardcoded id test (`buzz-agent` or `goose`). A remote
  harness whose id matches gets the LLM-provider selector; any other remote id does not, however
  the host's own catalog describes it.
- `BUZZ_ACP_TEAM_INSTRUCTIONS` is not carried to the host — the deploy payload has no team field, so
  a team-linked remote agent silently loses its team instructions.
- `MCP_HOOK_SERVERS` is not emitted. `mcp_hooks` is local catalog metadata the provider cannot
  compute, so remote agents have no `_Stop`/`_PostCompact` hook tools.
- `check` is implemented but has no desktop caller. `discover_harnesses` serves as the de facto
  preflight, since it is the first op the create flow runs against a host.
- The credential gate cannot see what the host already supplies. It asks the pinned REMOTE harness
  which env keys matter, but the runtime file layer (`~/.config/goose/config.yaml`) is local, so it
  is suppressed entirely for a remote create rather than answering for the wrong machine. A host
  whose config file already carries the credentials is therefore still asked for them. Closing this
  needs a `check`-style round trip that reports the host's own configuration — a protocol addition.
- The tailnet device picker filters out phones and TVs, but still offers Windows peers, which
  cannot be deploy targets. Picking one fails at deploy, not at selection.
- **Neither host-side tool is installed unless the desktop supplies one.** `deploy` installs the
  binaries named by `agent.buzz_acp_binary` / `agent.buzz_cli_binary` (from `BUZZ_ACP_PUSH_BINARY` /
  `BUZZ_CLI_PUSH_BINARY`) on a host that has none; with the variables unset a missing `buzz-acp` is
  still exit 90 and a missing `buzz` is still just a warning. Resolving the right release artifacts
  for the host — which is what makes the create dialog's deploy-will-install promise true, and what
  gives every remote agent CLI parity with a local one without a developer setting an env var — is
  the immediate follow-up, and the version-refresh rule rides with it.
- An already-installed tool is never upgraded by a deploy, by design (see the staleness rule). A host
  stuck on an old `buzz-acp` or `buzz` has to be updated by hand until artifact fetching lands.
