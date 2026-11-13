# Running Buzz on Windows with WSL2

`draft`

## Scope

This document covers running Buzz on a Windows host that uses **WSL2** for its
development environment. It is aimed at contributors and self-hosters whose
toolchains — Rust, Node, the vendor CLIs that back ACP harnesses — live inside a
Linux distro rather than on Windows itself.

It covers three things:

1. **Topology** — which Buzz components run on Windows, which run in WSL2, and
   why the boundary falls where it does.
2. **Setup** — Docker Desktop integration, port conflicts, and building the
   relay from a WSL2 clone.
3. **Agents** — installing harnesses so the desktop can find them, and running
   `buzz-acp` from inside WSL2 so agents operate on Linux-side repositories.

It is **not** a general installation guide; see the README for that. It assumes
WSL2 (not WSL1) and a working distro.

## Topology

Buzz splits cleanly across the Windows/WSL2 boundary, but the split is not the
obvious one. Two facts drive it:

- **The relay is not a container.** `docker-compose.yml` provisions backing
  services only — Postgres, Redis, MinIO, Keycloak, Adminer, Prometheus. The
  relay itself is a host binary (`just relay` → `cargo run -p buzz-relay`).
- **The desktop app is a native Windows binary.** Any agent it launches is a
  Windows process and sees the Windows filesystem.

The recommended arrangement:

| Component | Runs on | Rationale |
|---|---|---|
| Backing services (Docker) | WSL2 | Native Linux containers; no path-translation issues |
| Relay (`buzz-relay`) | WSL2 | Native toolchain via Hermit; no cross-compilation |
| Desktop client | Windows | Real GUI, tray, notifications, deep links |
| Agents (`buzz-acp`) | WSL2 *(see [Running agents in WSL2](#running-agents-in-wsl2))* | Access to Linux-side repositories and toolchains |

Clone into the WSL2 filesystem (for example `~/buzz`), **not** `/mnt/c`. Rust and
Node builds against the `/mnt/c` 9p mount are substantially slower, and git
operations on a `\\wsl.localhost\` path from Windows produce ownership and
line-ending problems.

WSL2 forwards `localhost` between Windows and the distro, so a relay bound to
`0.0.0.0:3000` inside WSL2 is reachable from the Windows desktop client at
`ws://localhost:3000` with no additional configuration. The default
`BUZZ_BIND_ADDR` in `.env.example` already binds all interfaces.

> If localhost forwarding stops working after a suspend/resume cycle, run
> `wsl --shutdown` from PowerShell and restart the distro. On Windows 11,
> `networkingMode=mirrored` under `[wsl2]` in `%UserProfile%\.wslconfig` avoids
> the problem entirely.

## Docker Desktop integration

Docker Desktop must be running **and** integrated with the specific distro that
holds the clone. Enabling "integration with default WSL distro" is not always
sufficient: Docker Desktop provisions `/var/run/docker.sock` into a distro when
integration is applied, and a distro that was already running when Docker
Desktop started may not receive it.

Symptom — the Docker CLI resolves but the daemon does not:

```console
$ docker info
failed to connect to the docker API at unix:///var/run/docker.sock:
dial unix /var/run/docker.sock: connect: no such file or directory
```

while the same engine answers from Windows:

```console
> docker.exe info --format '{{.ServerVersion}}'
29.3.1
```

Fix: **Settings → Resources → WSL Integration**, enable the distro explicitly,
then **Apply & Restart**. (`wsl --shutdown` followed by a restart also works, at
the cost of terminating every WSL session.)

Docker Desktop does not start on login by default. Any `just` recipe that
touches Docker will fail until it is running.

## Host port conflicts

`docker compose up` can **silently** fail to publish a port that is already
bound on the host, leaving the container running and healthy but unreachable.
This is easy to miss because the failure surfaces later as an unrelated error.

A common case on developer machines is a native PostgreSQL install holding
`127.0.0.1:5432`. The container starts, `pg_isready` passes inside the
container, and migrations then connect to the *native* server — which has no
`buzz` role:

```console
error: database error: password authentication failed for user "buzz"
```

Confirm by inspecting the published ports. An unpublished container shows a bare
container port with no host mapping:

```console
$ docker compose ps --format '{{.Name}}\t{{.Ports}}'
buzz-postgres   5432/tcp                                  ← not published
buzz-redis      0.0.0.0:6379->6379/tcp, [::]:6379->6379/tcp
```

Resolve it with a local, untracked override rather than editing the tracked
compose file:

```yaml
# docker-compose.override.yml
name: buzz
services:
  postgres:
    ports:
      - "5433:5432"
```

and point the environment at the new host port:

```dotenv
DATABASE_URL=postgres://buzz:buzz_dev@localhost:5433/buzz
PGPORT=5433
```

The same pattern applies to any of the other published ports — `6379`, `8082`,
`8180`, `9000`, `9001`, `9090`.

## Building and running the relay

The pinned toolchain comes from Hermit; no system-wide Rust, Node, or pnpm
install is required.

```bash
cd ~/buzz
export PATH="$PWD/bin:$PATH"

just setup     # starts services, runs migrations, seeds the local community
just relay     # cargo run -p buzz-relay  → binds 0.0.0.0:3000
```

Verify from both sides of the boundary — the Windows check is the one that
matters for the desktop client:

```bash
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:3000/health
/mnt/c/Windows/System32/curl.exe -s -o NUL -w '%{http_code}\n' http://localhost:3000/health
```

Both should return `200`.

## Installing agent harnesses

The desktop resolves a harness by looking up **two** binaries on `PATH`: the ACP
adapter and the vendor CLI it wraps (`readiness/cli_login.rs`). If either is
missing the harness reports as unavailable, and — because the readiness probe
also covers login state — a missing binary can surface in the UI as an
authentication problem rather than a missing dependency.

Because the desktop client is a Windows process, it scans the **Windows** `PATH`.
Harnesses intended for the desktop must therefore be installed on Windows, even
when the rest of the workflow lives in WSL2:

```console
> npm install -g @agentclientprotocol/claude-agent-acp
> npm install -g @anthropic-ai/claude-code
```

```console
> npm install -g @agentclientprotocol/codex-acp
> powershell -c "irm https://chatgpt.com/codex/install.ps1 | iex"
> codex login
```

Not every harness needs an adapter, and not every installer puts its binary
somewhere `PATH` already covers. The difference matters on Windows because the
desktop resolves harnesses by scanning `PATH` only:

| Harness | Adapter required | Installs to | On `PATH` after install |
|---|---|---|---|
| Claude Code | `claude-agent-acp` (npm) | `%APPDATA%\npm` | yes |
| Codex | `codex-acp` (npm) | `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin` | yes |
| Goose | none — native ACP | `%USERPROFILE%\.local\bin` | **no** — must be added |
| Grok Build | none — native ACP | `%USERPROFILE%\.grok\bin` | yes — installer adds it |
| Hermes Agent | none — `hermes-acp` ships with it | its own venv `Scripts` directory | yes |

Harnesses that speak ACP natively (`goose`, `grok`, `hermes-acp`) are a single
binary; the adapter-backed ones are two, and both must resolve.

When an installer does not modify `PATH`, add its directory to the **User**
scope rather than following the common `$env:PATH + ';…'` suggestion — that
form copies every Machine-scope entry into the user's own `PATH` and duplicates
them permanently:

```powershell
$userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
$target   = "$env:USERPROFILE\.local\bin"
if ($userPath -split ';' -notcontains $target) {
    [Environment]::SetEnvironmentVariable('PATH', "$userPath;$target", 'User')
}
```

npm's global prefix (`%APPDATA%\npm`) is on `PATH` by default, so both binaries
become resolvable. Two notes:

- **Restart the desktop app after installing a harness.** The availability scan
  is performed at startup and cached.
- A vendor CLI installed by another application may not be on `PATH` at all. The
  Claude desktop application, for example, installs its CLI under a
  version-pinned directory (`%APPDATA%\Claude\claude-code\<version>\claude.exe`)
  that no `PATH` scan will find. Installing the npm package alongside it
  provides a stable, resolvable entry point.

Verify:

```console
> where claude-agent-acp
> claude --version
```

## Running agents in WSL2

An agent launched by the Windows desktop is a Windows process. It sees `C:\` and
the Windows toolchain — not repositories or interpreters inside the distro. For
a WSL2-centric workflow this is usually the wrong environment.

Note also that the Windows shell resolver used by `buzz-dev-mcp` deliberately
**excludes** WSL's launcher when locating `bash.exe`, resolving Git Bash instead
(`crates/buzz-dev-mcp/src/shell.rs`). Pointing a desktop-managed harness at
`wsl.exe` is therefore working against the grain of the design.

The supported route is to launch the harness independently. As
[`remote-agents.md`](remote-agents.md) states, the desktop is *one launcher
among many*: what makes a process a live Buzz agent is a keypair, a NIP-OA auth
tag, and a relay URL handed to `buzz-acp` as environment. A shell script inside
WSL2 is a conforming launcher.

```bash
cd ~/buzz
export PATH="$PWD/bin:$PATH"
cargo build -p buzz-acp

# the adapter and vendor CLI must also be present inside the distro
npm install -g @agentclientprotocol/claude-agent-acp

BUZZ_RELAY_URL="wss://your-community.example" \
BUZZ_PRIVATE_KEY="<agent private key>" \
BUZZ_ACP_AGENT_COMMAND="claude-agent-acp" \
BUZZ_ACP_AGENT_OWNER="<owner public key>" \
  ./target/debug/buzz-acp
```

The agent process, the ACP adapter, and the vendor CLI all run inside WSL2 with
access to the Linux filesystem and toolchain. The relay may be local or remote;
only the agent's execution environment changes.

This composes with either topology: a relay running in the same distro, or a
hosted community reached over `wss://`.

## Troubleshooting

| Symptom | Cause | Resolution |
|---|---|---|
| `docker info` fails in WSL2 but works from Windows | Socket not provisioned into the distro | Enable the distro explicitly in WSL Integration, Apply & Restart |
| `password authentication failed for user "buzz"` | Port 5432 held by another service; container unpublished | Check `docker compose ps` for a bare `5432/tcp`; remap via override |
| Harness shows as not installed after signing in | Adapter or vendor CLI not on the Windows `PATH` | Install both via npm; restart the desktop app |
| `Access is denied` from `cmd.exe /c start …` | Working directory is a `\\wsl.localhost\` UNC path | `cd` to a Windows path first, or invoke the binary directly |
| `npm.cmd: line 13: unexpected EOF` | bash is interpreting a Windows `.cmd` | Invoke through `cmd.exe /c "cd /d C:\… && npm …"` |
| Desktop cannot reach a relay in WSL2 | localhost forwarding dropped after suspend | `wsl --shutdown` and restart, or enable mirrored networking |
| Agent cannot see Linux repositories | Agent launched by the Windows desktop | Run `buzz-acp` inside WSL2 (see above) |

## See also

- [`remote-agents.md`](remote-agents.md) — the launcher contract and agent lifecycle
- [`linux-rendering-troubleshooting.md`](linux-rendering-troubleshooting.md) — related platform notes
