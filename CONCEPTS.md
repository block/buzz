# Concepts

Shared domain vocabulary for this project - entities, named processes, and status concepts with project-specific meaning.
Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine.
Glossary only, not a spec or catch-all.

## Local Buzz Pilot

### Buzz Community
A workspace selected by the client-facing relay authority; tenant-observable state such as channels, messages, search, workflow state, media, git metadata, and audit history is community-local.

### Host Authority
The host and port portion of the relay URL that Buzz uses to resolve which community a local request belongs to.

### Active Pilot Community
Steve's current local Buzz pilot community for ongoing work and agent visibility.
For this local pilot, active continuity is intentionally on `localhost:3030`, not the upstream default `localhost:3000`.

### Archive Community
A preserved local Buzz community that remains available for read-only reference but is not the place where new pilot work should happen.
The old `localhost:3000` community is archive-only unless Steve approves a backup-first export or migration.

### Day 0 Summary
The continuity message posted into the active pilot community to carry forward enough prior context for the next agent or operator.
It is a summary, not a raw-message migration.

### Day 0 Channel Authority
The owner/admin membership posture that determines who can make privileged changes to the four durable Day 0 pilot channels.
For this pilot, the preferred steady state is one durable Steve-controlled manager pubkey with normal Buzz-authorized access across `buzz-pilot`, `install-support`, `repo-review`, and `agent-runs`.

### Agent Runs Channel
The canonical Buzz channel for task roots, blocker replies, closeouts, and handoffs during Steve's pilot.
It lives in the active `localhost:3030` community and is selected by `BUZZ_PILOT_AGENT_RUNS_CHANNEL_ID`.

### Smoke Check
A read-only startup verification that proves the active relay is ready and that expected pilot context is visible through the intended community authority.

### Slack Visibility Boundary
The rule that Slack is advisory visibility only for this pilot.
Buzz owns continuity and handoff memory; Slack may mirror sanitized status summaries but must not become the canonical record.
