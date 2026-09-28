# Bestie Coffee

RHS category that stores `/hula-coffee` briefing runs.

## Skill

- Skill lives on JM’s Mac at `~/.claude/skills/hula-coffee/SKILL.md` (not in this repo).
- Desktop invokes it by sending a **top-level** Bestie DM turn:
  ```
  [Bestie coffee]

  /hula-coffee
  ```
- Same path for scheduled morning runs and the **Brew** button.

## Schedule

- Default **08:00 local** (`Date#setHours` — America/Phoenix on JM’s machine).
- Prefs on `BestieCoffeeState.prefs` `{ hour, minute }` for a later settings UI.
- Fires only when relay presence is **`online`** (not away/offline).
- **Only two fire paths:** (1) explicit **Brew** click (2) once inside the local **08:00 + 5m window** if the day is not claimed. Wake / remount / presence / Jobs / NL must **not** catch up later (e.g. 08:38).
- At most one **scheduled** run per local calendar day — day is **claimed at begin** (and on missed-window) so remount/reload/wake cannot re-fire.
- Each fire is **source-tagged** in the prompt (`[Bestie coffee] · brew|scheduled`) and `console.info("[bestie-coffee] fire", …)`.
- Mutex: `pendingRun` + process-wide send lock; never re-send once `triggerMessageId` is bound. Brew shares the brewing lock.
- Do **not** also run macOS `com.hula.coffee-slack` / `/hula-coffee --install` launchd — that was a second 08:00 path sharing `~/.hula-coffee` cache with Assistant.
- Brew is manual, shares the brewing lock, and can run any time (unless a coffee turn is live).

## Brewing UI (live ACP turn)

Do **not** treat a sticky `pendingRun` spinner or ACP 👀 (“seen”) alone as brewing.

While `/hula-coffee` is actually in flight:

1. **Disable Brew**.
2. Show **🤔…** on the Coffee RHS tab and sheet (thinking face + ellipsis), not eyes-only.
3. Live = ACP agent-working on the Assistant DM for an **open coffee turn**, or the short post-start grace while the turn is still queued.
4. **Not** live for Thread Summarize, Jobs, Reminder notifies, or other Assistant turns — even if ACP is working on the Assistant DM. A newer summarize/job/reminder system turn supersedes coffee for the brewing indicator.

## Coffee tab capture

Every `/hula-coffee` outcome becomes a Coffee tab entry when the agent replies **in-thread to the coffee trigger** (success briefs, failures, NCP-disabled messages, other errors). Capture keys off the reply parent event id — never an arbitrary Assistant DM message (so Summarize replies cannot steal or fake a coffee entry). **Never** capture ACP kind:7 status reactions (`👀` / `💬`) — they share the trigger e-tag but are not the brew brief; writing capture from the same message loop that observes replies must filter to kind:9 / 40002 only.

The Assistant DM **channel window is roots-only**; agent replies are in-thread under the coffee trigger. `BestieWakeController` therefore loads the **thread-replies** subtree for open coffee triggers (and keeps a DM subscription even when the user is elsewhere) so Brew outcomes still land in the Coffee tab.

Abandoned `pendingRun` locks clear (and finalize a failure Coffee entry when a
trigger was posted) when:

1. ACP is idle past start grace + settle (~60s),
2. a newer competing system turn finished and ACP is idle, or
3. hard timeout (~8 min) even if the working signal is stuck.

**Path C upgrade:** open stubs (empty brief/output, abandoned timeout, or
`replyMessageId` null) under a coffee trigger root are replaced when the real
in-thread agent reply arrives — even if the trigger aged out of the roots-only
channel window (stubs seed `unmatchedCoffeeTriggers`; parent id match is
case-insensitive). Pending binds from the reply parent when the trigger id was
not set yet. **Path D:** if a pending brew’s agent reply lands top-level (no
`e` tag — first Assistant DM `--reply-to` omission), fold it into the pending
trigger so Coffee cannot stick on 👀. Capture loads thread-replies for every
stub trigger id.

Reload migrates stuck pending older than start grace. Brew click retries after
abandoning leftover pending so the button cannot silently no-op.

## Storage

`buzz-bestie-coffee.v1:<relay>:<owner>` (owner-scoped; legacy `…:agent` migrates once) — entries `{ id, ranAt, brief, fullOutput, source, triggerMessageId, replyMessageId }`, `forgottenTriggerIds` (deleted triggers must not rehydrate from chat), `pendingRun`, `lastScheduledDayKey`.

## Product rules

- Coffee / Jobs / Reminder notifies post with `parentEventId: null` (new top-level messages, never thread replies).
