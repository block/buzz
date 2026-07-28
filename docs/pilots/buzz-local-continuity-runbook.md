# Buzz Local Continuity Runbook
Created: 2026-07-26
Last verified: 2026-07-28 against active local relay `ws://localhost:3030`

## Purpose

Keep the local Buzz pilot recoverable and legible across restarts, port changes, and Codex sessions.

Use this runbook when:

- The Buzz Mac app is open but the local relay is not responding.
- A previous pilot conversation seems missing.
- A future agent needs to know which local Buzz community to use.
- Any command might affect local Buzz data.

## Current Canonical State

### Active Pilot Community

Use `localhost:3030` as the active Buzz pilot community.

Steve decided to avoid port `3000` while Buzz is being piloted, so `localhost:3000` is archive/reference only unless a backup-first migration or export is chosen later.

The active relay is currently expected on:

- Relay authority: `ws://localhost:3030`
- Health endpoint: `http://127.0.0.1:8088/_readiness`
- Metrics port: `9202`
- Community host: `localhost:3030`

### Recovered Old Community

The older local pilot community was recovered on its original authority and summarized into the active Day 0 community.

- Relay authority: `ws://localhost:3000`
- Health endpoint: `http://127.0.0.1:8080/_readiness`
- Community host: `localhost:3000`
- Channels:
  - `pilot-demo`: `d1ff7784-04a8-4bdf-801f-5a5e268a516e`
  - `codex-pilot-smoke`: `743676d7-6046-4b86-907f-4297ad01cdc1`
- Metadata verified: 2 channels, 44 events.
- Compact readback verified:
  - `pilot-demo` returned two compact messages.
  - `codex-pilot-smoke` returned an empty compact sample with `--limit 3`.
- Archive summary posted into active Day 0 `buzz-pilot`:
  - Event: `295d3891fb6a200a325f148ed651e4fc519f7b51f9d15bb9cad84b041871d8aa`

### Newer Day 0 Community

The newer Day 0 pilot community is the active pilot community:

- Relay authority used during Day 0: `ws://localhost:3030`
- Community host: `localhost:3030`
- Channels:
  - `buzz-pilot`: `3cdf4550-0501-4825-b54e-87213ea08b66`
  - `install-support`: `7cf15a6f-a601-4c40-92a3-5fee69594992`
  - `repo-review`: `577ef732-7ee7-44dd-bd3d-f2ef0473a286`
  - `agent-runs`: `d0bf00d9-e76d-44a8-bf4c-61725f79f3d4`
- Metadata verified: 4 channels, 180 events.
- Day 0 evidence lives in `docs/pilots/2026-07-25-buzz-day0-slack-visibility.md`.
- `buzz-pilot` was unarchived after a local Postgres backup so the archive summary could be posted.
- On Monday, July 27, 2026, all four Day 0 pilot channels were converted from 1-hour ephemeral rooms into durable local continuity channels. The active pilot should no longer depend on TTL renewal to keep `buzz-pilot`, `install-support`, `repo-review`, and `agent-runs` writable.

### Day 0 Channel Authority

Durable TTL alone is not enough; future operators also need a normal authority
path for privileged Day 0 channel maintenance.

Preferred steady state:

- One durable Steve-controlled manager pubkey is an `owner` or `admin` on all four Day 0 channels.
- Routine maintenance uses normal Buzz commands such as `buzz channels members`, `buzz channels add-member`, and `buzz channels update --no-ttl`.
- Direct local database repair remains fallback-only for this pilot.

Start with the read-only authority audit:

```bash
./scripts/audit-day0-channel-authority.sh
```

Status on Tuesday, July 28, 2026: the audit can read the active pilot database
and confirms all four Day 0 channels are permanent and active, but authority is
still fragmented across four sole-owner pubkeys with no admins. Run the repair
helper below before treating Day 0 authority as normalized.

If the audit shows the target manager is missing Day 0 authority, first try the
normal path with an already-authorized Buzz key. When that is not possible or
does not repair every channel, rerun with the documented local-only fallback:

```bash
export BUZZ_PILOT_PROOF_PRIVATE_KEY=<nsec-or-64-char-hex-private-key>
export BUZZ_PRIVATE_KEY="$BUZZ_PILOT_PROOF_PRIVATE_KEY"
export BUZZ_RELAY_URL=http://localhost:3030
./scripts/repair-day0-channel-authority.sh --allow-local-fallback
```

Fallback rules:

- If `--target-pubkey` is omitted, the helper derives the target pubkey from `BUZZ_PILOT_PROOF_PRIVATE_KEY` or `BUZZ_PRIVATE_KEY`.
- If the target pubkey is not yet a relay member for `localhost:3030`, `--allow-local-fallback` creates a backup and adds local relay membership before channel repair.
- The helper first attempts ordinary `buzz channels add-member` when a current authorized `BUZZ_PRIVATE_KEY` is present.
- Unless `--skip-proof` is set, the helper also needs a Buzz write identity in shell via `BUZZ_PILOT_PROOF_PRIVATE_KEY` or `BUZZ_PRIVATE_KEY`.
- If fallback is still required, the helper creates a fresh local Postgres backup before touching Day 0 channel membership.
- The helper finishes by proving the repaired path with `buzz channels update --no-ttl` on all four Day 0 channels.

Identity handling rule:

- Default to the temporary-shell path for Steve's pilot: reveal the existing Buzz Mac app identity from Settings > Profile > Private key, export it only in the terminal that will run the repair, then close that terminal when done.
- Do not write Steve's Buzz identity key into this repo's `.env`, tracked docs, examples, prompts, Slack, GitHub, screenshots, or shell history.
- A persistent local secret file such as `~/.buzz-local-secrets` is a later convenience tradeoff, not the default. Use it only if Steve explicitly chooses it and keep it owner-only (`chmod 600`).

### Canonical Choice For New Work

Use `localhost:3030` as the canonical local community while Buzz is being piloted.

Do not use port `3000` for active Buzz pilot work. Keep it available for other local apps and for archive-only recovery checks.

Do not switch ports casually. In Buzz local dev, `localhost:3000` and `localhost:3030` are separate communities, not aliases.

## Mental Model

Buzz local continuity has four separate layers:

| Layer | What It Means | How To Check |
| --- | --- | --- |
| Mac app | Client UI can be open without a relay | App window exists, but that alone proves no memory |
| Relay | Process serving Buzz over WebSocket/HTTP | `lsof` on port `3030` or readiness on `8088` |
| Docker services | Postgres, Redis, MinIO, and support services | `docker compose ps` |
| Postgres volume | Durable local event log | `docker volume ls --filter name=buzz` |

The durable memory is the relay-backed event log in Postgres, not the open Mac app.

## Safe Preflight

Run these checks before starting or recovering a local community.

```bash
lsof -nP -iTCP:3030 -sTCP:LISTEN
lsof -nP -iTCP:8088 -sTCP:LISTEN
docker compose ps
docker volume ls --filter name=buzz
```

Expected active-pilot state:

- Nothing else is listening on `3030`.
- Nothing else is listening on `8088`.
- `buzz-postgres` is healthy or startable.
- `buzz-postgres-data` exists.

If port `3000` is occupied by another app, leave that app alone and keep Buzz on `localhost:3030`.

For a quick read-only check of the current state, run:

```bash
./scripts/buzz-pilot-smoke.sh
```

The smoke check does not start services, edit the database, or inspect the archive community. It only verifies active relay readiness, active channel listing, and recent `buzz-pilot` readback through `localhost:3030`.

## Agent Update Helper

Use `scripts/post-pilot-agent-update.sh` for task-level pilot visibility.

Required environment:

- `BUZZ_RELAY_URL`
- `BUZZ_PRIVATE_KEY`
- `BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID`

Optional environment:

- `BUZZ_PILOT_SLACK_WEBHOOK_URL` for advisory-only Slack mirroring
- `BUZZ_PILOT_CHANNEL_ID_OVERRIDE` for explicit local testing only
- `BUZZ_PILOT_CLI` when the helper should use a specific local Buzz CLI binary

Normal root post:

```bash
scripts/post-pilot-agent-update.sh \
  --status started \
  --task-title "Short task title" \
  --summary "What just started" \
  --next-owner "Codex"
```

Normal reply update:

```bash
scripts/post-pilot-agent-update.sh \
  --status needs-steve \
  --task-title "Short task title" \
  --summary "Decision needed on relay startup path" \
  --next-owner "Steve" \
  --reply-to <root-event-id> \
  --changed docs/pilots/buzz-local-continuity-runbook.md
```

Status contract:

- Use `started` for the normal task root.
- Use `blocked`, `needs-steve`, `changed`, `handoff`, or `done` for replies.
- Slack is optional and advisory-only. Buzz remains canonical.
- Keep keys and webhook URLs in environment or untracked local secret storage only.
- Never post `localhost` URLs, raw logs, private keys, or webhook URLs into Slack.

## Steve-Local Startup Environment

Use this environment set for active pilot startup:

| Variable | Value | Why It Matters |
| --- | --- | --- |
| `RELAY_URL` | `ws://localhost:3030` | Seeds `localhost:3030` host rows during migration/bootstrap. |
| `BUZZ_BIND_ADDR` | `127.0.0.1:3030` | Binds the relay away from upstream default port `3000`. |
| `BUZZ_HEALTH_PORT` | `8088` | Moves health checks away from upstream default `8080`. |
| `BUZZ_METRICS_PORT` | `9202` | Keeps metrics off the default local metrics port. |
| `BUZZ_RELAY_URL` | `ws://localhost:3030` | Points desktop/dev clients at the active pilot relay. |

Why there are two relay URLs:

- `RELAY_URL` is read by local seeding and relay configuration.
- `BUZZ_RELAY_URL` is read by agent/client tooling and desktop dev environment wiring.
- The Buzz CLI can use `http://localhost:3030` for REST-style readback against the same active community.

Keep all of them aligned for Steve-local pilot work.

## Start And Stop Safely

### Start Relay Only On `localhost:3030`

Use relay-only recovery when the desktop app is already open.

```bash
RELAY_URL=ws://localhost:3030 \
BUZZ_BIND_ADDR=127.0.0.1:3030 \
BUZZ_HEALTH_PORT=8088 \
BUZZ_METRICS_PORT=9202 \
BUZZ_RELAY_URL=ws://localhost:3030 \
just relay
```

Expected signals:

- Config includes `bind_addr:"127.0.0.1:3030"`.
- Health probe listener starts on port `8088`.
- Relay logs `buzz-relay TCP listening`.

Note: the relay recipe may still log `relay_url:"ws://localhost:3000"` when `.env` still carries the upstream default or when a prior shell did not export `RELAY_URL`. The decisive verification is whether readiness succeeds on `8088` and `BUZZ_RELAY_URL=http://localhost:3030` lists the Day 0 channels.

Verify readiness:

```bash
curl --silent --show-error --fail http://127.0.0.1:8088/_readiness
```

Expected:

```json
{"status":"ready"}
```

### Start Full Desktop And Relay

Use full desktop launch only when you want Buzz to relaunch the Tauri app too.

```bash
RELAY_URL=ws://localhost:3030 \
BUZZ_BIND_ADDR=127.0.0.1:3030 \
BUZZ_HEALTH_PORT=8088 \
BUZZ_METRICS_PORT=9202 \
BUZZ_RELAY_URL=ws://localhost:3030 \
just dev
```

If the desktop app is already open, prefer `just relay` first.

### Stop Services Without Deleting Data

Stopping the relay process is safe.

Stopping Docker services without deleting named volumes is safe:

```bash
docker compose down
```

Do not use this if you need the relay to keep serving the Mac app right now.

## Never Run Without Backup

These are destructive or potentially destructive for this pilot:

- `docker compose down -v`
- `docker volume rm buzz-postgres-data`
- `scripts/dev-reset.sh`
- Any manual update to `communities`, `events`, `channels`, `channel_members`, or other community-scoped tables.
- Any attempt to merge `localhost:3000` and `localhost:3030` rows in-place.

If a command includes `-v`, `--volumes`, `volume rm`, `dev-reset`, or SQL updates to community identity, stop and back up first.

## Backup And Restore

### Backup Before Migration Or Host-Row Edits

Use a timestamped dump outside the repo.

```bash
mkdir -p ~/Backups/buzz
docker exec -e PGPASSWORD=buzz_dev buzz-postgres \
  pg_dump -U buzz -d buzz --format=custom \
  > ~/Backups/buzz/buzz-local-$(date +%Y%m%d-%H%M%S).dump
```

Do not store database dumps in the repo.

### Restore

Restores are intentionally not a routine pilot action.

Before restoring:

1. Stop and confirm no relay is writing to the database.
2. Preserve the current `buzz-postgres-data` volume or take a second backup.
3. Restore into a disposable database first when possible.
4. Only restore over the active local database after Steve explicitly approves.

## Verify Community Metadata

Use metadata checks before content reads.

```bash
docker exec -i -e PGPASSWORD=buzz_dev buzz-postgres \
  psql -U buzz -d buzz -v ON_ERROR_STOP=1 \
  -c "SELECT c.host, count(DISTINCT ch.id) AS channels, count(e.id) AS events FROM communities c LEFT JOIN channels ch ON ch.community_id = c.id LEFT JOIN events e ON e.community_id = c.id WHERE c.host IN ('localhost:3000','localhost:3030') GROUP BY c.host ORDER BY c.host;"
```

Expected current result:

| Host | Channels | Events |
| --- | --- | --- |
| `localhost:3000` | 2 | 44 |
| `localhost:3030` | 4 | 180 |

List channel metadata without message contents:

```bash
docker exec -i -e PGPASSWORD=buzz_dev buzz-postgres \
  psql -U buzz -d buzz -v ON_ERROR_STOP=1 \
  -c "SELECT c.host, ch.id, ch.name, ch.channel_type, ch.visibility, ch.created_at FROM communities c JOIN channels ch ON ch.community_id = c.id WHERE c.host IN ('localhost:3000','localhost:3030') ORDER BY c.host, ch.created_at;"
```

## Verify Active Community Through The CLI

Use a disposable key for read-only checks.

```bash
BUZZ_RELAY_URL=http://localhost:3030 \
BUZZ_PRIVATE_KEY=$(openssl rand -hex 32) \
/Users/Steve/dev/GitProjects/buzz/scripts/buzz --format compact channels list
```

Expected active Day 0 channels:

```json
[{"channel_id":"d0bf00d9-e76d-44a8-bf4c-61725f79f3d4","name":"agent-runs"},{"channel_id":"3cdf4550-0501-4825-b54e-87213ea08b66","name":"buzz-pilot"},{"channel_id":"577ef732-7ee7-44dd-bd3d-f2ef0473a286","name":"repo-review"},{"channel_id":"7cf15a6f-a601-4c40-92a3-5fee69594992","name":"install-support"}]
```

To verify the archive summary is visible in the active community:

```bash
BUZZ_RELAY_URL=http://localhost:3030 \
BUZZ_PRIVATE_KEY=$(openssl rand -hex 32) \
/Users/Steve/dev/GitProjects/buzz/scripts/buzz --format compact messages get \
  --channel 3cdf4550-0501-4825-b54e-87213ea08b66 \
  --limit 10
```

Expected: the result includes event `295d3891fb6a200a325f148ed651e4fc519f7b51f9d15bb9cad84b041871d8aa`.

## Verify Old Archive Community Through The CLI

Use a disposable key for read-only checks.

```bash
BUZZ_RELAY_URL=http://localhost:3000 \
BUZZ_PRIVATE_KEY=$(openssl rand -hex 32) \
/Users/Steve/dev/GitProjects/buzz/scripts/buzz --format compact channels list
```

Expected old-community channels:

```json
[{"channel_id":"d1ff7784-04a8-4bdf-801f-5a5e268a516e","name":"pilot-demo"},{"channel_id":"743676d7-6046-4b86-907f-4297ad01cdc1","name":"codex-pilot-smoke"}]
```

Bounded compact sample from `pilot-demo`:

```bash
BUZZ_RELAY_URL=http://localhost:3000 \
BUZZ_PRIVATE_KEY=$(openssl rand -hex 32) \
/Users/Steve/dev/GitProjects/buzz/scripts/buzz --format compact messages get \
  --channel d1ff7784-04a8-4bdf-801f-5a5e268a516e \
  --limit 3
```

Verified result: two compact messages returned.

Bounded compact sample from `codex-pilot-smoke`:

```bash
BUZZ_RELAY_URL=http://localhost:3000 \
BUZZ_PRIVATE_KEY=$(openssl rand -hex 32) \
/Users/Steve/dev/GitProjects/buzz/scripts/buzz --format compact messages get \
  --channel 743676d7-6046-4b86-907f-4297ad01cdc1 \
  --limit 3
```

Verified result: empty compact sample.

## Continue The `localhost:3030` Community

Use `localhost:3030` for active Buzz pilot work.

```bash
RELAY_URL=ws://localhost:3030 \
BUZZ_BIND_ADDR=127.0.0.1:3030 \
BUZZ_HEALTH_PORT=8088 \
BUZZ_METRICS_PORT=9202 \
BUZZ_RELAY_URL=ws://localhost:3030 \
just dev
```

If the local database returns `relay: no community is configured for this host`, seed or verify host rows before retrying.

## Agent Memory Rules

Buzz should be treated as a recoverable handoff log, not as automatic perfect memory.

Agents should:

- Read task roots and closeout replies first.
- Cite channel IDs and event IDs when relying on Buzz context.
- Prefer compact thread reads over raw transcript dumps.
- Treat setup chatter as provisional unless a closeout says what is true.
- Link GitHub branches, commits, PRs, or local docs instead of pasting long logs.

Agents should not:

- Paste secrets, `.env` values, private keys, auth tags, tokens, cookies, or webhook URLs.
- Treat Slack as canonical memory.
- Treat the Mac app being open as proof that relay memory is available.
- Merge local communities without a backup-first migration plan.

## Slack Visibility Boundary

Slack is advisory visibility only.

Use Slack for:

- Short lifecycle summaries.
- Blocker notices.
- Links to Buzz threads, local docs, branches, commits, or PRs.

Do not use Slack for:

- Raw logs.
- Secrets.
- Canonical task state.
- Final code review decisions.
- Anything that should survive as the authoritative task record. Put that in Buzz first.

Buzz owns handoff context. GitHub owns code state.

## Quick Recovery Checklist

1. Confirm `3030` and `8088` are free.
2. Confirm `buzz-postgres-data` exists.
3. Start relay-only with `RELAY_URL=ws://localhost:3030 BUZZ_BIND_ADDR=127.0.0.1:3030 BUZZ_HEALTH_PORT=8088 BUZZ_METRICS_PORT=9202 BUZZ_RELAY_URL=ws://localhost:3030 just relay`.
4. Run `./scripts/buzz-pilot-smoke.sh`.
5. If the smoke check fails readiness, verify the relay process and health port before changing data.
6. If the smoke check fails channel readback, inspect host rows for `localhost:3030`; do not switch to `3000`.
7. Treat `localhost:3000` as archive-only unless Steve explicitly asks for backup-first recovery, export, or migration.

## Current Open Choices

- Should repeated agent writes use a persistent disposable pilot identity stored outside the repo?
