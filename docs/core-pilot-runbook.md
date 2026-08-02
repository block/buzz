# Core Buzz local-pilot runbook

This pilot is a local, WSL-hosted research-and-drafting evaluation for **public
or synthetic data only**. It is not approved for live client work, deal teams,
MNPI, PII, attachments, or Azure deployment. The frozen scorecard in
`docs/core-pilot-scorecard.md` is the gate for any later decision.

## Prerequisites

Use Windows with Docker Desktop running and WSL available. Build the release
binaries once under the repository's Hermit environment; the scripts refuse to
build or install anything themselves. The required binaries are
`target/release/buzz-relay`, `buzz-acp`, and `buzz-agent`.

Copy `config/core-pilot/core-pilot.env.example` to a user-owned location outside
Git, such as `~/.config/core-buzz/pilot.env`. Update its one UUID channel and
its one banker-owner public key to the identities created for `Core Lab` and
`core-research`. Create a separate, restrictive (`chmod 600`) user-owned file
at `~/.config/core-buzz/agent.env` containing exactly:

```text
OPENAI_COMPAT_API_KEY=<locally managed OpenAI credential>
BUZZ_PRIVATE_KEY=<agent Nostr private key>
```

Do not paste either value into chat, a terminal command, or any checked-in
file. The scripts parse only those two secret records; they do not source the
file and never print secret values.

## Start and connect

From WSL at the repository root, run:

```bash
./scripts/core-pilot-preflight.sh
./scripts/core-pilot-start.sh
```

Preflight validates the fixed model/publishing restrictions, ownership/channel
scope, prompt file, secret-file permissions, and existing release binaries.
Start only runs `docker compose up -d postgres redis minio minio-init`, waits
for `http://127.0.0.1:3000/_readiness`, and launches the local relay and ACP
agent. It neither opens Buzz Desktop nor builds software.

Open the already-installed Windows Buzz Desktop and connect to
`ws://127.0.0.1:3000`. Use only `Core Lab` / `core-research` and synthetic
messages. The agent may make one cited reply to a qualifying message from the
configured owner, or remain silent.

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
