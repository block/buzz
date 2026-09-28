# Bestie Threads (Assistant RHS)

Threads tracked for Summarize + history. UI may say **Assistant**; storage /
protocol ids stay `bestie-thread*` / Bestie Threads.

## What appears in the list

1. **Agent participation** — any non–Assistant-DM thread where the assigned
   Assistant agent authored a message (discovered from home-feed
   `agent_activity`, plus Ask Assistant enrollments that already upsert).
2. **Ask Assistant** — opening Ask Assistant on a message enrolls that thread
   (`source: ask`).
3. **Manual +** — header `+` adds a thread the agent is **not** in yet, ensures
   read access, enrolls (`source: add`), and kicks off Summarize.

Remove (trash) only drops local tracking; it does not remove channel membership.

## Summarize

Summarize posts a **top-level Assistant DM** turn (`parentEventId: null`) with
`[Bestie thread summarize]` and channel / root / tracking ids so the agent can
call `buzz messages thread --channel <UUID> --event <ID>`.

### Live UI

Show **🤔…** on the Threads RHS category / row **only** while that summarize
turn is the latest open system turn on the Assistant DM — not while Coffee,
Jobs, or Reminder notifies own ACP. Same grace / stale-pending rules as Coffee
(idle settle ~60s, hard timeout ~8 min, reload clears stuck pending). When
pending is abandoned, write a failure `lastSummary` so the row is never stuck
on Summarizing… forever.

### Outcomes on the row

Every summarize reply that parents to the summarize trigger is stored on the
thread as `lastSummary` / `lastSummaryAt` — success briefs **and** failures /
errors. Coffee replies cannot steal a pending summarize.

Like Coffee, the channel window is roots-only — summarize outcomes are loaded
from the **thread-replies** cache for the summarize trigger root.

## ACL / relay rules (research)

Relay channel access for the **agent’s own** NIP-98 `/query` and WS REQ:

| Channel visibility | Agent is member? | Can read thread? |
|---|---|---|
| `open` | no | **Yes** — `get_accessible_channel_ids` unions all open channels; ingest `check_channel_membership` also allows open non-members. |
| `open` | yes | Yes |
| `private` | no | **No** — REQ/query return `restricted: not a channel member`. |
| `private` | yes | Yes |

Publish (reply in channel) uses the same membership-or-open gate.

DMs: membership is fixed at creation; do not invite into the Assistant DM via +.

## Smallest path that works (+ add)

1. Resolve `buzz://message?channel=&id=&thread=` (or channel id + root/event id).
2. Load channel details + members.
3. If **private** and agent is not a member → `add_channel_members` (invite bot).
   Open channels skip invite (read already allowed).
4. Upsert tracked thread + dispatch summarize.

If the user cannot add members to a private channel, surface that error and do
not enroll.
