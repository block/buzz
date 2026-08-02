---
title: Local Buzz Pilot Communities Are Selected By Host Authority
date: 2026-07-26
category: developer-experience
module: local-buzz-pilot
problem_type: developer_experience
component: development_workflow
severity: medium
applies_when:
  - "Running a local Buzz pilot on a non-default port"
  - "Recovering or comparing old local Buzz pilot conversations"
  - "Debugging apparent memory loss after switching relay URLs"
tags: [buzz-pilot, local-relay, community-authority, startup, continuity]
---

# Local Buzz Pilot Communities Are Selected By Host Authority

## Context

Steve's local Buzz pilot appeared to lose conversation continuity after switching away from the upstream default relay authority.
The underlying issue was not data loss: Buzz treats the request host authority as the community boundary, so `localhost:3000` and `localhost:3030` are separate local communities.

The current Steve-local pilot uses `localhost:3030` for active work, health on `8088`, and metrics on `9202`.
The older `localhost:3000` community remains an archive/reference community.

## Guidance

Use the read-only pilot smoke check before changing local data or starting another relay:

```bash
./scripts/buzz-pilot-smoke.sh
```

If the smoke check reports that the active relay is absent, start the Steve-local relay with the full environment so both relay binding and seeded community host rows point at `3030`:

```bash
RELAY_URL=ws://localhost:3030 \
BUZZ_BIND_ADDR=127.0.0.1:3030 \
BUZZ_HEALTH_PORT=8088 \
BUZZ_METRICS_PORT=9202 \
BUZZ_RELAY_URL=ws://localhost:3030 \
just relay
```

Do not switch active pilot work back to `3000` just because upstream docs or relay defaults mention it.
In this repo, upstream defaults remain valid for generic development; Steve-local pilot continuity overrides them only for this local bundle.

When checking whether old conversation is available, separate three cases:

- Active continuity lives in `localhost:3030` and is verified by reading the `buzz-pilot` channel.
- The old `localhost:3000` archive can still be inspected read-only through Postgres or by temporarily running the old authority when the port is free.
- Raw old archive messages have not been migrated into `localhost:3030`; the active channel contains a summary event instead.

## Why This Matters

Host-authority scoped communities make local port changes feel like memory loss even when the same database still contains both histories.
A future agent that follows generic `localhost:3000` startup snippets can accidentally inspect or seed the wrong community, then conclude the active pilot is empty.

The safest operator pattern is to treat startup as a verification problem, not a port-listener problem.
Readiness on `8088` proves the relay is alive, but the channel readback through `localhost:3030` proves the active community is the one Steve intends to pilot.

## When to Apply

- Before starting or restarting Steve's local Buzz pilot.
- Before using the Buzz Mac app to continue pilot conversations.
- Before querying, exporting, or migrating old local Buzz messages.
- When relay logs mention `ws://localhost:3000` but the active pilot should be on `3030`.

## Examples

The active pilot state can be verified without mutation:

```bash
./scripts/buzz-pilot-smoke.sh
```

Expected healthy signal:

```text
ok: active Buzz pilot is ready on http://localhost:3030; archive summary event is visible.
```

If old raw archive content is needed, query it as archive content first.
Do not merge the `localhost:3000` and `localhost:3030` communities in place without a fresh backup and an explicit migration plan.

## Related

- `AGENTS.md`
- `README.md`
- `docs/pilots/buzz-local-continuity-runbook.md`
- `docs/pilots/2026-07-25-buzz-day0-slack-visibility.md`
- `docs/dogfood-reports/2026-07-26-codex-fix-dev-startup-pilot-buzz-continuity-handoff.md`
- `scripts/buzz-pilot-smoke.sh`
- `scripts/seed-local-community.sh`
