# Core Buzz local-pilot runbook

This pilot is a local, WSL-hosted research-and-drafting evaluation for **public
or synthetic data only**. It is not approved for live client work, deal teams,
MNPI, PII, attachments, or Azure deployment. The frozen scorecard in
`docs/core-pilot-scorecard.md` is the gate for any later decision.

## Prerequisites

Use Windows with Docker Desktop running, Docker Compose v2.24.4 or newer, and
WSL available. Keep at least 40 GiB free on the Windows host before the first
native/release build; the WSL virtual disk can grow by roughly 27 GiB, and host
exhaustion can remount its ext4 filesystem read-only.

On a fresh Windows VM, open an elevated PowerShell window and install Ubuntu
for WSL. Restart Windows if the first command requests it, then reopen the
elevated window, update WSL, and install Docker Desktop:

```powershell
wsl.exe --install -d Ubuntu
wsl.exe --update
winget.exe install --exact --id Docker.DockerDesktop `
  --accept-source-agreements --accept-package-agreements
wsl.exe --list --verbose
```

Start Docker Desktop from the Windows Start menu. In Docker Desktop, select
**Settings > General > Use the WSL 2 based engine**, then select **Settings >
Resources > WSL Integration**, enable the Ubuntu distribution, and choose
**Apply & restart**. Docker Desktop supplies the Linux `docker` client and
Compose plugin to that distribution; do not install a second Docker Engine
inside Ubuntu. In the `wsl.exe --list --verbose` output above, confirm that
Ubuntu shows version `2`.

Open the Ubuntu terminal and install the host packages used by the build,
bootstrap, export, and import paths:

```bash
sudo apt-get update
sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  build-essential ca-certificates cmake coreutils curl file findutils gawk git \
  gnupg grep iproute2 jq libssl-dev nano openssl pinentry-curses pkg-config \
  procps sed xxd
```

This installs `gpg` through `gnupg`; `realpath`, `stat`, and `sha256sum`
through `coreutils`; `ss` through `iproute2`; and `pkill` through `procps`.
Verify every required command, Docker connectivity, and the minimum Compose
version before cloning or importing anything:

```bash
missing=0
for name in git jq gpg openssl xxd realpath stat sha256sum curl ss pkill \
  gcc g++ make cmake pkg-config docker; do
  command -v "$name" >/dev/null 2>&1 || {
    printf 'missing command: %s\n' "$name" >&2
    missing=1
  }
done
test "$missing" -eq 0 || exit 1
docker info >/dev/null || {
  printf 'Docker Desktop is not running or WSL integration is disabled\n' >&2
  exit 1
}
compose_version="$(docker compose version --short | \
  sed -E 's/^v?([0-9]+\.[0-9]+\.[0-9]+).*/\1/')"
printf '%s\n' "$compose_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || {
  printf 'could not parse Docker Compose version: %s\n' "$compose_version" >&2
  exit 1
}
dpkg --compare-versions "$compose_version" ge 2.24.4 || {
  printf 'Docker Compose %s is older than required 2.24.4\n' \
    "$compose_version" >&2
  exit 1
}
printf 'prerequisites ready (Docker Compose %s)\n' "$compose_version"
```

The five pilot binaries do not require the Linux GTK/WebKit packages used by
the Tauri desktop CI jobs. Install those separately from `CONTRIBUTING.md` only
if you intend to run the full desktop/Tauri checks in WSL.

In WSL, build the five required release binaries exactly once:

```bash
cd ~/src/buzz-core
. ./bin/activate-hermit
cargo build --release \
  -p buzz-relay \
  -p buzz-admin \
  -p buzz-cli \
  -p buzz-acp \
  -p buzz-agent
for binary in buzz-relay buzz-admin buzz buzz-acp buzz-agent; do
  test -x "target/release/$binary" || { printf 'missing: %s\n' "$binary" >&2; exit 1; }
done
```

The scripts never build or install software. Run the deterministic bootstrap:

```bash
./scripts/core-pilot-bootstrap.sh
```

Bootstrap starts only Postgres, Redis, MinIO, and MinIO initialization through
the Core Compose lock. Its four images are immutable-digest pinned and every
published backing-service port is bound to `127.0.0.1`; PostgreSQL uses host
port `15432` to avoid collisions with a Windows PostgreSQL service. Bootstrap runs
migrations, generates four stable keypairs once, closes relay membership,
creates the `Core Banker`, `Core Research Partner`, and `Synthetic Non-Owner`
profiles; and creates/reuses the private `core-research` and `core-control`
channels. It writes restrictive files to `~/.config/core-buzz` and
`~/.local/state/core-buzz`. Re-running it is safe and does not replace the
identities or channels.

The generated `~/.config/core-buzz/agent.env` has an intentionally empty
`OPENAI_COMPAT_API_KEY`. Open that file in a local WSL editor and fill only the
value after the equals sign:

```bash
chmod 700 ~/.config/core-buzz ~/.local/state/core-buzz
chmod 600 ~/.config/core-buzz/agent.env ~/.config/core-buzz/pilot.env \
  ~/.local/state/core-buzz/channels.env
${EDITOR:-nano} ~/.config/core-buzz/agent.env
```

Do not paste any secret into chat or a shell command. Do not rename, remove, or
manually regenerate the `CORE_*` identity records. The scripts allowlist and
parse the records without sourcing them or printing their values.

## Move the pilot to a new VM

Do not copy a development worktree as the migration mechanism. A worktree's
`.git` file points back to Git metadata elsewhere on the source VM, and build
outputs, dependency directories, logs, PID markers, and Docker data are neither
portable nor part of the pilot state. The supported transfer has two payloads:
an incremental Git bundle and a GPG-symmetric encrypted identity/channel record.
`SHA256SUMS` accompanies them so copy corruption can be detected.

On the source VM, stop the pilot, require a clean committed checkout, and create
the transfer under the WSL home directory. The destination path must be absolute,
outside the repository, and nonexistent. Do not create it first, and do not use
`/mnt/c`: DrvFS permission mapping may be too permissive for private-state checks.

```bash
cd ~/src/buzz-core
./scripts/core-pilot-stop.sh
git status --short
transfer="$HOME/core-pilot-transfer-$(date +%Y%m%d-%H%M%S)"
./scripts/core-pilot-export.sh --output "$transfer"
(cd "$transfer" && sha256sum --check --strict SHA256SUMS)
printf 'Record separately — expected source commit: %s\n' "$(git rev-parse HEAD)"
printf 'Record separately — expected bundle SHA-256: %s\n' \
  "$(sha256sum "$transfer/core-pilot.bundle" | awk '{print $1}')"
```

GPG requests the symmetric passphrase through pinentry. Use a strong unique
passphrase and keep it separate from the transfer. Never put it in an argument or
environment variable. For controlled automation, both scripts accept
`--passphrase-fd N` for an already-open descriptor numbered 3 or higher.

Export fails if `agent.env` or `channels.env` is absent. That means no portable
pilot identity exists yet; do not invent placeholder state. For a source-only
move, create a transfer directory containing the committed branch bundle, its
exact source commit, and checksums, then let bootstrap create the first
identities on the new VM. There is no prior identity or channel continuity to
preserve in that case. Record the printed expected source commit separately
from the copied payload; it is the trusted value to compare on the new VM.

```bash
cd ~/src/buzz-core
base=b7bb15122e8a2053b545dc2210afc167f6c7a626
transfer="$HOME/core-pilot-source-transfer-$(date +%Y%m%d-%H%M%S)"
mkdir -m 700 "$transfer"
test -z "$(git status --porcelain --untracked-files=no)"
git merge-base --is-ancestor "$base" HEAD
git rev-parse HEAD > "$transfer/SOURCE_COMMIT"
git bundle create "$transfer/core-pilot.bundle" HEAD "^$base"
git bundle verify "$transfer/core-pilot.bundle"
(cd "$transfer" && sha256sum core-pilot.bundle SOURCE_COMMIT > SHA256SUMS)
(cd "$transfer" && sha256sum --check --strict SHA256SUMS)
printf 'Record separately — expected source commit: %s\n' "$(cat "$transfer/SOURCE_COMMIT")"
printf 'Record separately — expected bundle SHA-256: %s\n' \
  "$(sha256sum "$transfer/core-pilot.bundle" | awk '{print $1}')"
```

After export completes, the encrypted directory may be copied through Windows
Explorer from `\\wsl.localhost\<distribution>\home\<wsl-user>\...` to the secure
transport. On the new VM, copy the complete directory into the new WSL user's
home, then restore restrictive permissions. Before fetching or building bundle
code, compare its digest and `HEAD` with the two values recorded separately on
the source VM. A co-located `SHA256SUMS` detects copy damage but is not proof of
provenance.

```bash
transfer="$HOME/core-pilot-transfer"
chmod 700 "$transfer"
find "$transfer" -maxdepth 1 -type f -exec chmod 600 {} +
(cd "$transfer" && sha256sum --check --strict SHA256SUMS)
test "$(sha256sum "$transfer/core-pilot.bundle" | awk '{print $1}')" = \
  '<expected-bundle-sha256-from-source-VM>'
test "$(git bundle list-heads "$transfer/core-pilot.bundle" HEAD | awk '{print $1}')" = \
  '<expected-source-commit-from-source-VM>'
```

For a source-only transfer, also compare `SOURCE_COMMIT` with the value recorded
separately on the source VM before using the bundle:

```bash
test "$(cat "$transfer/SOURCE_COMMIT")" = '<expected-source-commit-from-source-VM>'
```

Create a fresh public checkout and make the bundle prerequisite available. The
default prerequisite is commit
`b7bb15122e8a2053b545dc2210afc167f6c7a626`; it is also recorded in both the
bundle header and encrypted metadata. A normal full clone of `block/buzz`
contains it. Fetch the incremental `HEAD` into a new local branch:

```bash
git clone https://github.com/block/buzz.git ~/src/buzz-core
cd ~/src/buzz-core
git cat-file -e b7bb15122e8a2053b545dc2210afc167f6c7a626^{commit}
git bundle verify "$transfer/core-pilot.bundle"
git fetch "$transfer/core-pilot.bundle" HEAD:refs/heads/core-pilot-restored
git switch core-pilot-restored
if test -f "$transfer/SOURCE_COMMIT"; then
  test "$(git rev-parse HEAD)" = "$(cat "$transfer/SOURCE_COMMIT")"
fi
```

Build fresh dependencies and release binaries; do not transfer `target`,
`node_modules`, Hermit caches, release binaries, Docker volumes, uploaded media,
or message history:

```bash
. ./bin/activate-hermit
cargo build --release \
  -p buzz-relay \
  -p buzz-admin \
  -p buzz-cli \
  -p buzz-acp \
  -p buzz-agent
```

Import private state only when `core-pilot-state.gpg` exists, and only after
checking out the bundle commit. Import verifies the bundle
prerequisite and tip, exact source commit, reviewed-prompt hash, artifact
checksums, schema, identity fields, and channel UUIDs before it writes anything.
It creates current-user-owned mode-0600 `agent.env` and `channels.env`, leaves
the OpenAI key empty, is safe to repeat before the destination credential is
filled, and refuses to replace different existing state. Once the local API key
is populated, a repeat import deliberately refuses rather than overwriting it.

```bash
if test -f "$transfer/core-pilot-state.gpg"; then
  ./scripts/core-pilot-import.sh --source "$transfer"
fi
./scripts/core-pilot-bootstrap.sh
```

Bootstrap reconstructs fresh Postgres/Redis/MinIO state, relay membership,
profiles, channels using the imported UUIDs, and channel memberships. It does
not restore messages, media, logs, processes, or Docker data. Stop and investigate
instead of deleting anything if the new VM already has Buzz Docker volumes or
different pilot state.

Before retiring the old VM, compare only the non-secret public keys and channel
UUIDs, enter the OpenAI credential locally with the editor procedure above, then
run preflight/start and the synthetic scope checks. Keep the old VM and encrypted
transfer until the restored identities, both exact channel UUIDs, relay/channel
memberships, Desktop connection, and restart behavior are verified.

Install Buzz Desktop fresh on the destination Windows VM; do not copy the
installed executable directory from the old VM. Use the same approved release
from <https://github.com/block/buzz/releases>, verify its digest and scan it with
the destination's Windows Security policy, then install it. Before using the
helper below, confirm that both `%LOCALAPPDATA%\Buzz\buzz-desktop.exe` and
`%LOCALAPPDATA%\Buzz\buzz.exe` exist. The source tree, bundle, and Linux release
build do not install these Windows files.

## Start and connect

From WSL at the repository root, run:

```bash
./scripts/core-pilot-preflight.sh
./scripts/core-pilot-start.sh
```

Preflight validates all five binaries, exact model/publishing restrictions,
stable identity/channel bindings, the reviewed prompt hash, secret-file
metadata, and the OpenAI credential gate. Start runs only the four locked
services through the Core Compose override, verifies the relay, and
does not report ready until one eager agent pool is initialized, ACP is
connected, at least two memberships are discovered, only `core-research` is
subscribed, and online presence is published.

Useful non-secret checks are:

```bash
curl -fsS http://127.0.0.1:3000/_readiness >/dev/null && printf 'relay ready\n'
tail -n 100 ~/.local/state/core-buzz/bootstrap.log
tail -n 100 ~/.local/state/core-buzz/relay.log
tail -n 100 ~/.local/state/core-buzz/acp.log
```

Verify and launch the already-installed, digest- and Defender-checked Buzz
Desktop from Windows PowerShell using the checked-in helper. It targets the installed
`buzz-desktop.exe`, verifies both Desktop and CLI installation files, refuses
an existing single-instance process, and confirms the new process remains
running before it clears the banker environment and opens the Core Lab link:

```powershell
$Distro = 'Ubuntu'
$WslUser = (wsl.exe -d $Distro -- whoami).Trim()
$Script = "\\wsl.localhost\$Distro\home\$WslUser\src\buzz-core\scripts\core-pilot-desktop.ps1"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File $Script `
  -WslDistribution $Distro
```

In the Desktop add-community screen, confirm the prefilled relay and the name
`Core Lab`, then select `core-research`. Do not import the banker key into the
normal Desktop profile; the launch above uses shared ephemeral identity mode.
Use only synthetic/public messages. `core-control` exists solely to verify that
the agent does not subscribe or reply outside `core-research`.

## Stop and restart

Run `./scripts/core-pilot-stop.sh` to stop only the relay and ACP processes
whose pilot-owned markers were created by the launcher. It deliberately leaves
Docker services and their volumes intact. Re-run start to restart; a repeated
start with both pilot processes alive is idempotent.

Never run `docker compose down -v`, `just reset`, `scripts/dev-reset.sh`, or
any destructive reset for this pilot. Those can remove local state. If the
credential is absent or invalid, leave the stack stopped at the preflight gate;
do not substitute a different model/provider or weaken the policy.

## Evaluation and escalation

Record only synthetic/public test prompts and outcomes in the scorecard. Stop
the evaluation immediately for a hard-fail event. No Azure deployment, live
client data, attachments, or external communication is in scope unless Core
reviews a passing frozen evaluation and explicitly approves a new phase.

If the OpenAI key is absent, bootstrap may still complete through relay,
identity, profile, and channel setup. `core-pilot-preflight.sh` and ACP launch
must remain blocked. Do not enable lazy-pool startup or substitute another
provider/model. Stop the owned relay with `./scripts/core-pilot-stop.sh`; Docker
volumes remain intact.
