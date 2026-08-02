# Buzz Pilot Day 0 And Slack Visibility Notes
Created: 2026-07-25

## Purpose

Track the first execution pass for the Buzz agent-handoff pilot and start a low-risk Slack visibility path for agent activity.

Source plan: `docs/plans/2026-07-24-buzz-agent-handoff-pilot-plan.md`

## Day 0 State

### Verified

- Local helper bundle exists at `/Users/Steve/dev/GitProjects/buzz`.
- Upstream checkout exists at `/Users/Steve/dev/GitProjects/buzz/upstream`.
- Project-local helper scripts exist in `/Users/Steve/dev/GitProjects/buzz/scripts`.
- `target/release/buzz` exists and is executable.
- `just` is installed at `/opt/homebrew/bin/just`.
- `.env` exists in the upstream checkout. Its contents were not printed or copied.
- `BUZZ_RELAY_URL` and `BUZZ_PRIVATE_KEY` are not set in this shell.
- Docker Desktop is running and reachable.
- Docker services started successfully for Buzz local development.
- Database migrations completed.
- Buzz relay became ready on the non-conflicting local port set.
- All four pilot channels were created.
- Channel-level CLI read/write/search passed with a temporary dev key.
- Task-thread CLI compact readback passed with a disposable generated key.
- The older `localhost:3000` pilot community was summarized into the active Day 0 `buzz-pilot` channel.

### Port And Relay Findings

- Port `3000` is occupied by a Node/Next app that serves the Amancha Wedding site, not a Buzz relay.
- `curl http://127.0.0.1:3000` failed in one probe, while the Buzz CLI received an Amancha Wedding 404 response in another. Either way, port `3000` is not a trustworthy Buzz relay target.
- `just dev` should not be run on the default port while that app is active.
- The alternate relay port requires matching community host rows. The local database was seeded with `localhost:3030` and `127.0.0.1:3030` after the first CLI attempt returned `relay: no community is configured for this host`.
- On 2026-07-26, Steve decided to avoid port `3000` for active Buzz pilot work. Use `localhost:3030` for the pilot and treat `localhost:3000` as archive/reference only unless a backup-first export or migration is explicitly chosen.

Use this Buzz launch command for active pilot work:

```bash
RELAY_URL=ws://localhost:3030 \
BUZZ_BIND_ADDR=127.0.0.1:3030 \
BUZZ_HEALTH_PORT=8088 \
BUZZ_METRICS_PORT=9202 \
BUZZ_RELAY_URL=ws://localhost:3030 \
just dev
```

### Launch Result

First launch attempt, before Docker was running, failed with:

```text
failed to connect to the docker API at unix:///Users/Steve/.docker/run/docker.sock
```

After Docker started, `just dev` completed service startup, migrations, relay build, relay launch, frontend dev-server launch, desktop compile, and desktop app launch.

Relay readiness:

```text
GET http://127.0.0.1:8088/_readiness -> {"status":"ready"}
```

Relay listener:

```text
buzz-relay listening on 127.0.0.1:3030
```

Frontend dev server:

```text
http://localhost:20241/
```

Desktop app:

```text
target/debug/buzz-desktop launched
desktop identity generated
media proxy listening on 127.0.0.1:58636
Pocket TTS model ready at /Users/Steve/.buzz/models/pocket-tts
STT model ready at /Users/Steve/.buzz/models/parakeet-tdt-ctc-110m-en
```

First-run note: desktop launch downloaded speech/TTS model assets before declaring the TTS and STT models ready. This can make the first successful launch feel like it is stuck even after the bee screen appears.

### Pilot Channels

| Channel | Channel ID | Status |
| --- | --- | --- |
| `buzz-pilot` | `3cdf4550-0501-4825-b54e-87213ea08b66` | Created; Day 0 summary posted |
| `install-support` | `7cf15a6f-a601-4c40-92a3-5fee69594992` | Created |
| `repo-review` | `577ef732-7ee7-44dd-bd3d-f2ef0473a286` | Created |
| `agent-runs` | `d0bf00d9-e76d-44a8-bf4c-61725f79f3d4` | Created; first task root posted |

### CLI Evidence

Channel-level check:

- Relay: `http://localhost:3030`
- Identity source: temporary dev key
- Posted event: `975f2633bb6c67d56142d473a05a5a7a5b177e8a11a51fe6c6d184e62fa7b2b0`
- Result: message posted, read in recent messages, and found by search.

Day 0 summary:

- Channel: `buzz-pilot`
- Event: `2b423648a53999058a93b88d86c20a1eef06264b5760d618c8be1674d43d50aa`
- Result: posted and read back.

First agent task root:

- Channel: `agent-runs`
- Event: `f2d19a1a6ff0d2c3961164c438e16fe7c82dfbddb98e53d77ab2beed6460a066`
- Result: posted, found by search, and read back with `buzz --format compact messages thread`.

First agent task closeout:

- Channel: `agent-runs`
- Root event: `f2d19a1a6ff0d2c3961164c438e16fe7c82dfbddb98e53d77ab2beed6460a066`
- Closeout event: `18ea6e776b4798bd8da6a91fb17a1398c252b86b3ef47fc766329fd9f1a075c4`
- Result: posted as a threaded reply with outcome, changed artifacts, verification, remaining risks, and next owner.

Older `localhost:3000` community archive summary:

- Channel: `buzz-pilot`
- Event: `295d3891fb6a200a325f148ed651e4fc519f7b51f9d15bb9cad84b041871d8aa`
- Result: old `localhost:3000` channels and compact recoverable content were summarized into the active Day 0 channel.
- Source community: `localhost:3000`
- Source channels: `pilot-demo` (`d1ff7784-04a8-4bdf-801f-5a5e268a516e`) and `codex-pilot-smoke` (`743676d7-6046-4b86-907f-4297ad01cdc1`)
- Source metadata: 2 channels, 44 events.
- Safety note: `buzz-pilot` had been archived, and CLI unarchive with a disposable key was unauthorized. A local Postgres backup was taken under `/Users/Steve/Backups/buzz/` before clearing `archived_at` only for `buzz-pilot` in the `localhost:3030` community.
- Visibility note: old raw messages were not migrated into `localhost:3030`. The active `buzz-pilot` channel contains the summary event above; the raw old events remain in the `localhost:3000` archive community unless Steve approves backup-first export or migration later.
- Latest read-only archive check on 2026-07-26: `localhost:3000` was not reachable through the CLI because no relay was listening on port `3000`, but Postgres still held archive rows. The recoverable old content is thin: `pilot-demo` has 11 channel-scoped events with 4 non-empty content fields, including two `Hello from the Buzz pilot example.` messages; `codex-pilot-smoke` has 9 channel-scoped events with 2 non-empty system/archive content fields. Treat the active Day 0 summary as the useful continuity record unless Steve asks for a backup-first export.
- Durability note on Monday, July 27, 2026: the active `localhost:3030` Day 0 channels were changed from 1-hour TTL rooms to durable pilot channels so they stop auto-archiving during normal inactivity.

### Next Day 0 Actions

- Confirm the app window is connected to the relay after first-run model setup completes.
- Continue using `3030/8088/9202` for active Buzz pilot work.
- Use `./scripts/buzz-pilot-smoke.sh` for read-only startup verification.
- Use `scripts/post-pilot-agent-update.sh` for `agent-runs` task visibility posts.
- Treat the Day 0 pilot channels as durable continuity rooms, not ephemeral scratch channels.
- Avoid port `3000` during the pilot; use it only for archive/reference checks after confirming no unrelated app is using it.
- Create a persistent disposable pilot identity outside the repo if repeated agent writes should share one identity.
- Use a disposable pilot identity; do not paste or persist private key material in Buzz, Slack, GitHub, docs, or prompts.

## Slack Visibility Track

### Goal

Give Steve lightweight visibility into agent work without making Slack canonical, without replacing Buzz, and without exposing raw logs or secrets.

Buzz remains the system of record for agent handoff context during the pilot. Slack should receive status summaries and links only.

### Canonical Helper Contract

Use `scripts/post-pilot-agent-update.sh` as the single local helper for agent task visibility.

- Canonical Buzz destination: `BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID`
- Test-only override: `BUZZ_PILOT_CHANNEL_ID_OVERRIDE`
- Root status: `started`
- Reply statuses: `blocked`, `needs-steve`, `changed`, `handoff`, `done`
- Optional Slack visibility: `BUZZ_PILOT_SLACK_WEBHOOK_URL`
- Secrets boundary: env or other untracked local secret storage only

The helper should post to Buzz first, then optionally mirror a sanitized summary to Slack.
Slack is only for visibility. Buzz owns continuity, thread state, and handoff memory.

### Recommended Sequence

1. Start outbound-only.
2. Mirror only major agent lifecycle events.
3. Keep one Slack thread per Buzz task.
4. Add richer Slack API behavior only if outbound summaries prove useful.
5. Do not add Slack command/control until Buzz handoff trails are consistently useful.

### Phase 1: Incoming Webhook Mirror

Use an incoming webhook for the first Slack experiment because it is the smallest surface area: one secret URL posts JSON payloads into one selected channel.

Recommended Slack destination: `#buzz-agent-visibility` or an equivalent private pilot channel.

Mirror these events:

- Task requested in Buzz.
- Agent started.
- Main checkpoint found.
- Severe blocker.
- Task closed.
- PR or commit link added.

Do not mirror:

- Raw install logs.
- `.env` values.
- Private keys, auth tags, tokens, cookies, or webhook URLs.
- Long terminal transcripts.
- Final code review decisions that should stay canonical in GitHub.

Suggested Slack-safe message shape:

```markdown
*Buzz agent update:* <status>
Task: <short title>
Buzz: <non-secret Buzz reference>
GitHub: <branch, commit, or PR link if available>
Risk: <none | blocker | needs Steve>
```

### Phase 2: Bot Token Posting

Move from incoming webhook to Slack Web API posting only when we need richer control such as threaded replies, message updates, deletions, channel selection, or app-home messages.

Minimum expected scope: `chat:write`.

Extra caution:

- Store the bot token outside the repo.
- Rate-limit status updates; Slack generally allows about one message per second per channel for `chat.postMessage`.
- Keep Buzz and GitHub links as the canonical navigation path.

### Phase 3: Bidirectional Slack Events

Use Slack Events API or Socket Mode only after the pilot proves outbound visibility is valuable.

Potential later behaviors:

- Mention the Slack app to ask for the latest Buzz task status.
- Convert a Slack message into a Buzz task request after explicit confirmation.
- Notify when an agent asks for Steve's input.

Do not add bidirectional Slack controls in week one. They introduce auth, routing, and trust-boundary complexity before we know whether Slack visibility is useful.

### Security And Governance

- Treat Slack webhook URLs and bot tokens as secrets.
- Redact before posting to Slack using the same checklist as Buzz.
- Make Slack messages link-oriented, not transcript-oriented.
- Keep Slack advisory: Buzz owns handoff context, GitHub owns code state.
- If a sensitive artifact is posted to Slack, rotate the Slack secret and remove or redact the message if possible.

### Pilot Success Signal For Slack

Slack visibility is useful only if Steve can answer these questions faster:

- Which agent tasks are currently active?
- Which ones are blocked?
- Which ones changed code or docs?
- Which ones need Steve to review or decide?

If Slack adds another inbox without reducing rehydration time, keep it out of the week-one workflow.

## References

- Slack incoming webhooks: https://api.slack.com/messaging/webhooks
- Slack `chat.postMessage`: https://api.slack.com/methods/chat.postMessage
- Slack Events API: https://api.slack.com/apis/connections/events-api
