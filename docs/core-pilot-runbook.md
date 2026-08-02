# Core Buzz local-pilot runbook

This pilot is a local, WSL-hosted research-and-drafting evaluation for **public
or synthetic data only**. It is not approved for live client work, deal teams,
MNPI, PII, attachments, or Azure deployment. The frozen scorecard in
`docs/core-pilot-scorecard.md` is the gate for any later decision.

## Prerequisites

Use Windows with Docker Desktop running and WSL available. In WSL, build the
five required release binaries exactly once:

```bash
cd /home/blake/src/buzz-core-core-pilot
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

Bootstrap starts only Postgres, Redis, MinIO, and MinIO initialization; runs
migrations; generates four stable keypairs once; closes relay membership;
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

## Start and connect

From WSL at the repository root, run:

```bash
./scripts/core-pilot-preflight.sh
./scripts/core-pilot-start.sh
```

Preflight validates all five binaries, exact model/publishing restrictions,
stable identity/channel bindings, the reviewed prompt hash, secret-file
metadata, and the OpenAI credential gate. Start runs only
`docker compose up -d postgres redis minio minio-init`, verifies the relay, and
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

Verify the relay from Windows PowerShell, then launch the already-installed,
signed Buzz Desktop with the stable banker identity. Close Buzz first because
an existing single-instance process will not inherit the Core environment:

```powershell
Invoke-WebRequest -UseBasicParsing 'http://127.0.0.1:3000/_readiness' | Out-Null
if (Get-Process -Name Buzz -ErrorAction SilentlyContinue) {
  throw 'Close the existing Buzz process, then run this block again.'
}
$banker = (& wsl.exe -d Ubuntu -- bash -lc `
  'awk -F= ''$1=="CORE_BANKER_PRIVATE_KEY" {printf "%s",$2}'' "$HOME/.config/core-buzz/agent.env"').Trim()
if ([string]::IsNullOrWhiteSpace($banker)) { throw 'Core banker identity is unavailable.' }
try {
  $env:BUZZ_PRIVATE_KEY = $banker
  $env:BUZZ_SHARE_IDENTITY = '1'
  $env:BUZZ_RELAY_URL = 'ws://127.0.0.1:3000'
  Start-Process -FilePath "$env:LOCALAPPDATA\Buzz\Buzz.exe"
} finally {
  Remove-Item Env:\BUZZ_PRIVATE_KEY -ErrorAction SilentlyContinue
  Remove-Item Env:\BUZZ_SHARE_IDENTITY -ErrorAction SilentlyContinue
  Remove-Item Env:\BUZZ_RELAY_URL -ErrorAction SilentlyContinue
  $banker = $null
}
Start-Process 'buzz://add-community?relay=ws%3A%2F%2F127.0.0.1%3A3000&name=Core%20Lab'
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
